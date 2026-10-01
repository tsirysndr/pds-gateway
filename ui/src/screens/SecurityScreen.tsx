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

const passwordOnly = z.object({ password: z.string().min(1, "Enter your password") });
const codeOnly = z.object({
  code: z.string().trim().length(6, "Six digits from your authenticator"),
});
const passwordAndCode = passwordOnly.extend(codeOnly.shape);

export function SecurityScreen() {
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
      ? { Icon: IconShieldCheck, label: "On", color: "success" as const }
      : state === "pending"
        ? { Icon: IconShieldOff, label: "Finish setup", color: "warning" as const }
        : { Icon: IconShieldOff, label: "Off", color: "default" as const };

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
    mutationFn: (values: z.infer<typeof passwordAndCode>) =>
      disableTotp(client, values.password, values.code),
    onSuccess: () => {
      setEnrolling(null);
      refresh();
    },
  });
  const recovery = useMutation({
    mutationFn: (values: z.infer<typeof passwordAndCode>) =>
      regenerateRecovery(client, values.password, values.code),
  });

  if (status.data?.state === "unsupported" || isUnsupported(status.error)) {
    return (
      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h2 className="text-lg font-semibold">Two-factor authentication</h2>
        </CardHeader>
        <CardBody>
          <Alert tone="info">
            This server does not implement <code>social.rocksky.auth</code>.
            Two-factor is a server feature rather than part of the atproto
            lexicon, so it is only available where the PDS offers it.
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
            <h2 className="text-lg font-semibold">Two-factor authentication</h2>
            <p className="text-sm text-foreground-500">
              A code from your authenticator, on top of your password.
            </p>
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
              label="Set up"
              isPending={begin.isPending}
              error={begin.error}
              onSubmit={(values) => begin.mutate(values.password)}
            />
          )}

          {enrolling?.uri && (
            <div className="flex flex-col items-start gap-3">
              <p className="text-sm">
                Scan this with your authenticator, then enter the code it shows.
              </p>
              <div className="rounded-medium bg-white p-3">
                <QRCodeSVG value={enrolling.uri} size={168} />
              </div>
              {enrolling.secret && (
                <div className="w-full">
                  <p className="mb-1 text-xs text-foreground-500">
                    Or enter this secret by hand
                  </p>
                  <Snippet size="sm" hideSymbol className="w-full overflow-x-auto">
                    {enrolling.secret}
                  </Snippet>
                </div>
              )}
            </div>
          )}

          {(state === "pending" || enrolling) && (
            <CodeForm
              label="Confirm"
              isPending={confirm.isPending}
              error={confirm.error}
              onSubmit={(values) => confirm.mutate(values.code)}
            />
          )}

          {state === "enabled" && (
            <>
              <Divider />
              <p className="text-sm font-medium">Recovery codes</p>
              <PasswordCodeForm
                label="Generate new codes"
                isPending={recovery.isPending}
                error={recovery.error}
                onSubmit={(values) => recovery.mutate(values)}
              />
              {recovery.data?.recoveryCodes && (
                <Alert tone="success" title="Store these somewhere safe">
                  <div className="mt-1 grid grid-cols-2 gap-1 font-mono text-xs">
                    {recovery.data.recoveryCodes.map((code) => (
                      <span key={code}>{code}</span>
                    ))}
                  </div>
                </Alert>
              )}

              <Divider />
              <p className="text-sm font-medium">Turn off</p>
              <PasswordCodeForm
                label="Disable"
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
  onSubmit: (values: z.infer<typeof passwordOnly>) => void;
}) {
  const form = useForm<z.infer<typeof passwordOnly>>({
    resolver: zodResolver(passwordOnly),
    defaultValues: { password: "" },
  });
  return (
    <form className="flex flex-col gap-3" onSubmit={form.handleSubmit(onSubmit)}>
      <Field
        label="Password"
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
  onSubmit: (values: z.infer<typeof codeOnly>) => void;
}) {
  const form = useForm<z.infer<typeof codeOnly>>({
    resolver: zodResolver(codeOnly),
    defaultValues: { code: "" },
  });
  return (
    <form className="flex flex-col gap-3" onSubmit={form.handleSubmit(onSubmit)}>
      <Field
        label="Code"
        inputMode="numeric"
        autoComplete="one-time-code"
        placeholder="123456"
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
  onSubmit: (values: z.infer<typeof passwordAndCode>) => void;
}) {
  const form = useForm<z.infer<typeof passwordAndCode>>({
    resolver: zodResolver(passwordAndCode),
    defaultValues: { password: "", code: "" },
  });
  return (
    <form className="flex flex-col gap-3" onSubmit={form.handleSubmit(onSubmit)}>
      <div className="flex flex-col gap-3 sm:flex-row">
        <Field
          className="sm:flex-1"
          label="Password"
          type="password"
          autoComplete="current-password"
          error={form.formState.errors.password}
          {...form.register("password")}
        />
        <Field
          className="sm:w-40"
          label="Code"
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
