import { Button, Divider, Link } from "@heroui/react";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useMutation } from "@tanstack/react-query";
import { useAtomValue } from "jotai";
import { AuthCard } from "../components/AuthCard";
import { Alert, ErrorAlert } from "../components/Alert";
import { Field } from "../components/Field";
import { PasswordField } from "../components/PasswordField";
import { PdsSelect } from "../components/PdsSelect";
import { pdsUrlAtom } from "../atoms/store";
import { createClient } from "../lib/xrpc";

const makeSchema = (t: (key: string) => string) =>
  z
    .object({
      token: z.string().trim().min(1, t("reset.errorCode")),
      password: z.string().min(8, t("signup.errorPasswordLength")),
      confirmPassword: z.string().min(1, t("signup.errorConfirmRequired")),
    })
    .refine((values) => values.password === values.confirmPassword, {
      path: ["confirmPassword"],
      message: t("signup.errorConfirmMatch"),
    });

type Values = z.infer<ReturnType<typeof makeSchema>>;

/// Takes the emailed code and a new password, and sends them to the account's
/// own PDS — the one the forgot screen resolved from the handle, correctable
/// below. The code is the whole proof; no session is involved.
export function ResetScreen() {
  const { t } = useTranslation();
  const pds = useAtomValue(pdsUrlAtom);

  const form = useForm<Values>({
    resolver: zodResolver(makeSchema(t)),
    defaultValues: { token: "", password: "", confirmPassword: "" },
  });

  const reset = useMutation({
    mutationFn: (values: Values) =>
      createClient(pds).post("com.atproto.server.resetPassword", {
        token: values.token.trim(),
        password: values.password,
      }),
  });

  return (
    <AuthCard
      title={t("reset.title")}
      subtitle={t("reset.subtitle")}
      footer={
        <>
          <Link href="#/login" size="sm">{t("forgot.backToSignIn")}</Link>
        </>
      }
    >
      {reset.isSuccess ? (
        <>
          <Alert tone="success">{t("reset.done")}</Alert>
          <Button as={Link} href="#/login" color="primary" fullWidth>
            {t("common.signIn")}
          </Button>
        </>
      ) : (
        <>
          <form
            className="flex flex-col gap-4"
            onSubmit={form.handleSubmit((values) => reset.mutate(values))}
          >
            <Field
              label={t("reset.code")}
              autoComplete="one-time-code"
              error={form.formState.errors.token}
              {...form.register("token")}
            />
            <PasswordField
              label={t("reset.newPassword")}
              autoComplete="new-password"
              error={form.formState.errors.password}
              {...form.register("password")}
            />
            <PasswordField
              label={t("reset.confirmPassword")}
              autoComplete="new-password"
              error={form.formState.errors.confirmPassword}
              {...form.register("confirmPassword")}
            />
            {reset.error ? <ErrorAlert error={reset.error} /> : null}
            <Button type="submit" color="primary" isLoading={reset.isPending} fullWidth>
              {t("reset.submit")}
            </Button>
          </form>

          <Divider />
          <PdsSelect label={t("reset.serverLabel")} />
        </>
      )}
    </AuthCard>
  );
}
