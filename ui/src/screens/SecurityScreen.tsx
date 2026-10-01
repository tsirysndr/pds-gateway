import { useState } from "react";
import {
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  Divider,
  Snippet,
} from "@heroui/react";
import { QRCodeSVG } from "qrcode.react";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { IconShieldCheck, IconShieldOff } from "@tabler/icons-react";
import { Alert, ErrorAlert } from "../components/Alert";
import { Field } from "../components/Field";
import { useClient } from "../lib/api";
import {
  beginTotp,
  confirmTotp,
  disableTotp,
  fetchTotpStatus,
  isUnsupported,
  regenerateRecovery,
  type Enrollment,
} from "../lib/security";

const passwordOnly = (t: (key: string) => string) =>
  z.object({ password: z.string().min(1, t("security.errorPasswordRequired")) });
const codeOnly = (t: (key: string) => string) =>
  z.object({ code: z.string().trim().length(6, t("security.errorCodeLength")) });
const passwordAndCode = (t: (key: string) => string) =>
  passwordOnly(t).extend(codeOnly(t).shape);

export function SecurityScreen() {
  const { t } = useTranslation();
  const client = useClient();
  const queries = useQueryClient();
  const [enrolling, setEnrolling] = useState<Enrollment | null>(null);

  const status = useQuery({
    queryKey: ["totp", client.base],
    retry: false,
    queryFn: () => fetchTotpStatus(client),
  });

  const state = enrolling?.state ?? status.data?.state ?? "disabled";
  const badge =
    state === "enabled"
      ? { Icon: IconShieldCheck, label: t("security.statusOn"), color: "success" as const }
      : state === "pending"
        ? { Icon: IconShieldOff, label: t("security.statusFinishSetup"), color: "warning" as const }
        : { Icon: IconShieldOff, label: t("security.statusOff"), color: "default" as const };

  const refresh = () => queries.invalidateQueries({ queryKey: ["totp", client.base] });

  const begin = useMutation({
    mutationFn: (password: string) => beginTotp(client, password),
    onSuccess: (next) => setEnrolling(next),
  });
  const confirm = useMutation({
    mutationFn: (code: string) => confirmTotp(client, code),
    onSuccess: () => {
      setEnrolling(null);
      refresh();
    },
  });
  const disable = useMutation({
    mutationFn: (values: z.infer<ReturnType<typeof passwordAndCode>>) =>
      disableTotp(client, values.password, values.code),
    onSuccess: () => {
      setEnrolling(null);
      refresh();
    },
  });
  const recovery = useMutation({
    mutationFn: (values: z.infer<ReturnType<typeof passwordAndCode>>) =>
      regenerateRecovery(client, values.password, values.code),
  });

  if (status.data?.state === "unsupported" || isUnsupported(status.error)) {
    return (
      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h2 className="text-lg font-semibold">{t("security.title")}</h2>
        </CardHeader>
        <CardBody>
          <Alert tone="info">
            {t("security.notImplementedBefore")} <code>social.rocksky.auth</code>.{" "}
            {t("security.notImplementedAfter")}
          </Alert>
        </CardBody>
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="border border-default-200">
        <CardHeader className="items-center justify-between">
          <div className="flex flex-col gap-1">
            <h2 className="text-lg font-semibold">{t("security.title")}</h2>
            <p className="text-sm text-foreground-500">{t("security.subtitle")}</p>
          </div>
          <Chip
            color={badge.color}
            variant="flat"
            startContent={<badge.Icon size={16} />}
          >
            {badge.label}
          </Chip>
        </CardHeader>

        <CardBody className="gap-4">
          {state === "disabled" && (
            <PasswordForm
              label={t("security.setUp")}
              isPending={begin.isPending}
              error={begin.error}
              onSubmit={(values) => begin.mutate(values.password)}
            />
          )}

          {enrolling?.uri && (
            <div className="flex flex-col items-start gap-3">
              <p className="text-sm">{t("security.scanThis")}</p>
              <div className="rounded-medium bg-white p-3">
                <QRCodeSVG value={enrolling.uri} size={168} />
              </div>
              {enrolling.secret && (
                <div className="w-full">
                  <p className="mb-1 text-xs text-foreground-500">{t("security.orSecret")}</p>
                  <Snippet size="sm" hideSymbol className="w-full overflow-x-auto">
                    {enrolling.secret}
                  </Snippet>
                </div>
              )}
            </div>
          )}

          {state === "pending" && !enrolling && (
            <Alert tone="info" title={t("security.pendingTitle")}>
              {t("security.pendingBody")}
            </Alert>
          )}

          {(state === "pending" || enrolling) && (
            <CodeForm
              label={t("security.confirm")}
              isPending={confirm.isPending}
              error={confirm.error}
              onSubmit={(values) => confirm.mutate(values.code)}
            />
          )}

          {state === "pending" && !enrolling && (
            <>
              <Divider />
              <p className="text-sm font-medium">{t("security.startAgain")}</p>
              <p className="text-sm text-foreground-500">{t("security.startAgainBody")}</p>
              <PasswordForm
                label={t("security.newQr")}
                isPending={begin.isPending}
                error={begin.error}
                onSubmit={(values) => begin.mutate(values.password)}
              />
            </>
          )}

          {state === "enabled" && (
            <>
              <Divider />
              <p className="text-sm font-medium">{t("security.recoveryCodes")}</p>
              <PasswordCodeForm
                label={t("security.generateCodes")}
                isPending={recovery.isPending}
                error={recovery.error}
                onSubmit={(values) => recovery.mutate(values)}
              />
              {recovery.data?.recoveryCodes && (
                <Alert tone="success" title={t("security.codesTitle")}>
                  <div className="mt-1 grid grid-cols-2 gap-1 font-mono text-xs">
                    {recovery.data.recoveryCodes.map((code) => (
                      <span key={code}>{code}</span>
                    ))}
                  </div>
                </Alert>
              )}

              <Divider />
              <p className="text-sm font-medium">{t("security.turnOff")}</p>
              <PasswordCodeForm
                label={t("security.disable")}
                color="danger"
                isPending={disable.isPending}
                error={disable.error}
                onSubmit={(values) => disable.mutate(values)}
              />
            </>
          )}
        </CardBody>
      </Card>
    </div>
  );
}

function PasswordForm({
  label,
  isPending,
  error,
  onSubmit,
}: {
  label: string;
  isPending: boolean;
  error: unknown;
  onSubmit: (values: z.infer<ReturnType<typeof passwordOnly>>) => void;
}) {
  const { t } = useTranslation();
  const form = useForm<z.infer<ReturnType<typeof passwordOnly>>>({
    resolver: zodResolver(passwordOnly(t)),
    defaultValues: { password: "" },
  });
  return (
    <form className="flex flex-col gap-3" onSubmit={form.handleSubmit(onSubmit)}>
      <Field
        label={t("common.password")}
        type="password"
        autoComplete="current-password"
        error={form.formState.errors.password}
        {...form.register("password")}
      />
      {error ? <ErrorAlert error={error} /> : null}
      <Button type="submit" color="primary" isLoading={isPending} className="self-start">
        {label}
      </Button>
    </form>
  );
}

function CodeForm({
  label,
  isPending,
  error,
  onSubmit,
}: {
  label: string;
  isPending: boolean;
  error: unknown;
  onSubmit: (values: z.infer<ReturnType<typeof codeOnly>>) => void;
}) {
  const { t } = useTranslation();
  const form = useForm<z.infer<ReturnType<typeof codeOnly>>>({
    resolver: zodResolver(codeOnly(t)),
    defaultValues: { code: "" },
  });
  return (
    <form className="flex flex-col gap-3" onSubmit={form.handleSubmit(onSubmit)}>
      <Field
        label={t("login.factorLabel")}
        inputMode="numeric"
        autoComplete="one-time-code"
        placeholder={t("login.factorPlaceholder")}
        error={form.formState.errors.code}
        {...form.register("code")}
      />
      {error ? <ErrorAlert error={error} /> : null}
      <Button type="submit" color="primary" isLoading={isPending} className="self-start">
        {label}
      </Button>
    </form>
  );
}

function PasswordCodeForm({
  label,
  color = "primary",
  isPending,
  error,
  onSubmit,
}: {
  label: string;
  color?: "primary" | "danger";
  isPending: boolean;
  error: unknown;
  onSubmit: (values: z.infer<ReturnType<typeof passwordAndCode>>) => void;
}) {
  const { t } = useTranslation();
  const form = useForm<z.infer<ReturnType<typeof passwordAndCode>>>({
    resolver: zodResolver(passwordAndCode(t)),
    defaultValues: { password: "", code: "" },
  });
  return (
    <form className="flex flex-col gap-3" onSubmit={form.handleSubmit(onSubmit)}>
      <div className="flex flex-col gap-3 sm:flex-row">
        <Field
          className="sm:flex-1"
          label={t("common.password")}
          type="password"
          autoComplete="current-password"
          error={form.formState.errors.password}
          {...form.register("password")}
        />
        <Field
          className="sm:w-40"
          label={t("login.factorLabel")}
          inputMode="numeric"
          autoComplete="one-time-code"
          error={form.formState.errors.code}
          {...form.register("code")}
        />
      </div>
      {error ? <ErrorAlert error={error} /> : null}
      <Button type="submit" color={color} isLoading={isPending} className="self-start">
        {label}
      </Button>
    </form>
  );
}
