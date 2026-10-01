import { Button, Divider, Link } from "@heroui/react";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useAtomValue, useSetAtom } from "jotai";
import { AuthCard } from "../components/AuthCard";
import { Alert, ErrorAlert } from "../components/Alert";
import { Field } from "../components/Field";
import { HandleField } from "../components/HandleField";
import { PasswordField } from "../components/PasswordField";
import { PdsSelect } from "../components/PdsSelect";
import { pdsUrlAtom, sessionAtom } from "../atoms/store";
import { describeServer } from "../lib/servers";
import { createClient } from "../lib/xrpc";
import type { Session } from "../lib/types";

const makeSchema = (t: (key: string) => string) =>
  z
    .object({
      handle: z.string().trim().min(1, t("signup.errorHandle")),
      email: z.string().trim().email(t("signup.errorEmail")),
      password: z.string().min(8, t("signup.errorPasswordLength")),
      confirmPassword: z.string().min(1, t("signup.errorConfirmRequired")),
      inviteCode: z.string().trim().optional(),
    })
    // A typo in a password you cannot see locks you out of a new account, and
    // there is nothing to recover from yet.
    .refine((values) => values.password === values.confirmPassword, {
      path: ["confirmPassword"],
      message: t("signup.errorConfirmMatch"),
    });

type Values = z.infer<ReturnType<typeof makeSchema>>;

export function SignupScreen() {
  const { t } = useTranslation();
  const pds = useAtomValue(pdsUrlAtom);
  const setSession = useSetAtom(sessionAtom);

  const { data: server } = useQuery({
    queryKey: ["describeServer", pds],
    queryFn: () => describeServer(pds),
  });

  const domain = server?.availableUserDomains?.[0] ?? "";

  const form = useForm<Values>({
    resolver: zodResolver(makeSchema(t)),
    defaultValues: {
      handle: "",
      email: "",
      password: "",
      confirmPassword: "",
      inviteCode: "",
    },
  });

  const create = useMutation({
    mutationFn: (values: Values) =>
      createClient(pds).post<Session>("com.atproto.server.createAccount", {
        // A bare label is completed with the server's own domain, which is what
        // the signup form offers.
        handle: values.handle.includes(".")
          ? values.handle
          : `${values.handle}${domain}`,
        email: values.email,
        password: values.password,
        ...(values.inviteCode ? { inviteCode: values.inviteCode } : {}),
      }),
    onSuccess: (session) => setSession(session),
  });

  return (
    <AuthCard
      title={t("signup.title")}
      subtitle={domain ? t("signup.handlesEndIn", { domain }) : undefined}
      footer={
        <>
          {t("signup.alreadyHaveOne")} <Link href="#/login" size="sm">{t("common.signIn")}</Link>
        </>
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={form.handleSubmit((values) => create.mutate(values))}
      >
        <HandleField
          label={t("common.handle")}
          placeholder={t("signup.handlePlaceholder")}
          endContent={
            domain && <span className="text-small text-default-400">{domain}</span>
          }
          error={form.formState.errors.handle}
          {...form.register("handle")}
        />
        <Field
          label={t("common.email")}
          type="email"
          autoComplete="email"
          error={form.formState.errors.email}
          {...form.register("email")}
        />
        <PasswordField
          label={t("common.password")}
          autoComplete="new-password"
          error={form.formState.errors.password}
          {...form.register("password")}
        />
        <PasswordField
          label={t("signup.confirmPassword")}
          autoComplete="new-password"
          error={form.formState.errors.confirmPassword}
          {...form.register("confirmPassword")}
        />
        {server?.inviteCodeRequired && (
          <Field
            label={t("signup.inviteCode")}
            error={form.formState.errors.inviteCode}
            {...form.register("inviteCode")}
          />
        )}

        {create.error && <ErrorAlert error={create.error} />}

        <Button type="submit" color="primary" isLoading={create.isPending} fullWidth>
          {t("signup.title")}
        </Button>
      </form>

      <Divider />

      <div className="flex flex-col gap-2">
        <PdsSelect label={t("signup.createOn")} />
        <Alert tone="info">
          {t("signup.placement")}
        </Alert>
      </div>
    </AuthCard>
  );
}
