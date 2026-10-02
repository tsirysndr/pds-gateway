import { Button, Link } from "@heroui/react";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useMutation } from "@tanstack/react-query";
import { useAtomValue, useSetAtom } from "jotai";
import { AuthCard } from "../components/AuthCard";
import { Alert, ErrorAlert } from "../components/Alert";
import { Field } from "../components/Field";
import { HandleField } from "../components/HandleField";
import { lastHandleAtom, pdsUrlAtom } from "../atoms/store";
import { detectPds } from "../lib/resolve";
import { preferOrigin } from "../lib/servers";
import { createClient } from "../lib/xrpc";

const makeSchema = (t: (key: string) => string) =>
  z.object({
    handle: z
      .string()
      .trim()
      .min(1, t("login.errorHandleRequired"))
      .refine(
        (v) => v.includes(".") || v.startsWith("did:"),
        t("login.errorHandleShape"),
      ),
    email: z.string().trim().email(t("signup.errorEmail")),
  });

type Values = z.infer<ReturnType<typeof makeSchema>>;

/// Requests a password reset. The handle comes first because it is what
/// locates the server to ask; the email is where that server sends the link,
/// and the answer is the same whether the address exists or not.
export function ForgotScreen() {
  const { t } = useTranslation();
  const lastHandle = useAtomValue(lastHandleAtom);
  const setPds = useSetAtom(pdsUrlAtom);

  const form = useForm<Values>({
    resolver: zodResolver(makeSchema(t)),
    defaultValues: { handle: lastHandle, email: "" },
  });

  const request = useMutation({
    mutationFn: async (values: Values) => {
      const resolution = await detectPds(values.handle);
      const base = preferOrigin(resolution.service);
      setPds(base);
      await createClient(base).post("com.atproto.server.requestPasswordReset", {
        email: values.email,
      });
    },
  });

  return (
    <AuthCard
      title={t("forgot.title")}
      subtitle={t("forgot.subtitle")}
      footer={
        <>
          <Link href="#/login" size="sm">{t("forgot.backToSignIn")}</Link>
        </>
      }
    >
      {request.isSuccess ? (
        <>
          <Alert tone="success">{t("forgot.sent")}</Alert>
          <Button as={Link} href="#/reset" variant="bordered" fullWidth>
            {t("forgot.haveCode")}
          </Button>
        </>
      ) : (
        <form
          className="flex flex-col gap-4"
          onSubmit={form.handleSubmit((values) => request.mutate(values))}
        >
          <HandleField
            label={t("common.handle")}
            placeholder={t("login.handlePlaceholder")}
            autoComplete="username"
            error={form.formState.errors.handle}
            {...form.register("handle")}
          />
          <Field
            label={t("forgot.email")}
            type="email"
            autoComplete="email"
            error={form.formState.errors.email}
            {...form.register("email")}
          />
          {request.error ? <ErrorAlert error={request.error} /> : null}
          <Button type="submit" color="primary" isLoading={request.isPending} fullWidth>
            {t("forgot.submit")}
          </Button>
        </form>
      )}
    </AuthCard>
  );
}
