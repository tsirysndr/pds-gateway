import { Button, Card, CardBody, CardHeader, Chip, Code, Divider, Snippet } from "@heroui/react";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useAtomValue } from "jotai";
import { sessionAtom } from "../atoms/store";
import { Alert, ErrorAlert } from "../components/Alert";
import { Field } from "../components/Field";
import {
  useRequestEmailConfirmation,
  useSession,
  useUpdateHandle,
} from "../lib/api";

const makeSchema = (t: (key: string) => string) =>
  z.object({
    handle: z
      .string()
      .trim()
      .min(3, t("account.errorHandleLength"))
      .refine((v) => v.includes("."), t("account.errorHandleShape")),
  });

export function AccountScreen() {
  const { t } = useTranslation();
  const stored = useAtomValue(sessionAtom);
  const { data, error, isPending } = useSession();
  const account = data ?? stored;

  const rename = useUpdateHandle();
  const confirmEmail = useRequestEmailConfirmation();

  const form = useForm<z.infer<ReturnType<typeof makeSchema>>>({
    resolver: zodResolver(makeSchema(t)),
    defaultValues: { handle: account?.handle ?? "" },
  });

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="border border-default-200">
        <CardHeader className="flex-col items-start gap-1">
          <h2 className="text-lg font-semibold">{account?.handle ?? t("account.title")}</h2>
          <Code size="sm">{account?.did}</Code>
        </CardHeader>
        <CardBody className="gap-3">
          {error && <ErrorAlert error={error} />}
          {isPending && !stored && <p className="text-sm text-foreground-500">{t("account.loading")}</p>}

          <div className="flex flex-wrap items-center gap-2 text-sm">
            <span className="text-foreground-500">{t("account.emailTitle")}</span>
            <span>{account?.email ?? "—"}</span>
            {account?.email && (
              <Chip
                size="sm"
                variant="flat"
                color={account.emailConfirmed ? "success" : "warning"}
              >
                {account.emailConfirmed ? t("account.confirmed") : t("account.unconfirmed")}
              </Chip>
            )}
            {account?.email && !account.emailConfirmed && (
              <Button
                size="sm"
                variant="flat"
                isLoading={confirmEmail.isPending}
                onPress={() => confirmEmail.mutate()}
              >
                {t("account.sendConfirmation")}
              </Button>
            )}
          </div>
          {confirmEmail.isSuccess && (
            <Alert tone="success">{t("account.confirmationSent")}</Alert>
          )}
          {confirmEmail.error && <ErrorAlert error={confirmEmail.error} />}
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h3 className="font-medium">{t("account.changeHandle")}</h3>
        </CardHeader>
        <CardBody>
          <form
            className="flex flex-col gap-3 sm:flex-row sm:items-start"
            onSubmit={form.handleSubmit((values) => rename.mutate(values.handle))}
          >
            <Field
              className="sm:flex-1"
              label={t("account.newHandle")}
              autoCapitalize="none"
              spellCheck="false"
              error={form.formState.errors.handle}
              {...form.register("handle")}
            />
            <Button
              type="submit"
              color="primary"
              className="sm:mt-3"
              isLoading={rename.isPending}
            >
              {t("account.update")}
            </Button>
          </form>
          {rename.isSuccess && <Alert tone="success">{t("account.handleUpdated")}</Alert>}
          {rename.error && <ErrorAlert error={rename.error} />}
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h3 className="font-medium">{t("account.export")}</h3>
        </CardHeader>
        <CardBody className="gap-2">
          <p className="text-sm text-foreground-500">{t("account.exportBody")}</p>
          <Snippet size="sm" hideSymbol className="w-full overflow-x-auto">
            {`com.atproto.sync.getRepo?did=${account?.did ?? ""}`}
          </Snippet>
          <Divider />
          <p className="text-xs text-foreground-500">{t("account.deactivateNote")}</p>
        </CardBody>
      </Card>
    </div>
  );
}
