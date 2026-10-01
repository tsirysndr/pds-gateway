import { Button, Divider, Link } from "@heroui/react";
import { useForm } from "react-hook-form";
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

const schema = z
  .object({
    handle: z.string().trim().min(1, "Choose a handle"),
    email: z.string().trim().email("Enter a valid email"),
    password: z.string().min(8, "Use at least 8 characters"),
    confirmPassword: z.string().min(1, "Repeat your password"),
    inviteCode: z.string().trim().optional(),
  })
  // A typo in a password you cannot see locks you out of a new account, and
  // there is nothing to recover from yet.
  .refine((values) => values.password === values.confirmPassword, {
    path: ["confirmPassword"],
    message: "Both passwords must match",
  });

type Values = z.infer<typeof schema>;

export function SignupScreen() {
  const pds = useAtomValue(pdsUrlAtom);
  const setSession = useSetAtom(sessionAtom);

  const { data: server } = useQuery({
    queryKey: ["describeServer", pds],
    queryFn: () => describeServer(pds),
  });

  const domain = server?.availableUserDomains?.[0] ?? "";

  const form = useForm<Values>({
    resolver: zodResolver(schema),
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
      title="Create account"
      subtitle={domain ? `Handles end in ${domain}` : undefined}
      footer={
        <>
          Already have one? <Link href="#/login" size="sm">Sign in</Link>
        </>
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={form.handleSubmit((values) => create.mutate(values))}
      >
        <HandleField
          label="Handle"
          placeholder="alice"
          endContent={
            domain && <span className="text-small text-default-400">{domain}</span>
          }
          error={form.formState.errors.handle}
          {...form.register("handle")}
        />
        <Field
          label="Email"
          type="email"
          autoComplete="email"
          error={form.formState.errors.email}
          {...form.register("email")}
        />
        <PasswordField
          label="Password"
          autoComplete="new-password"
          error={form.formState.errors.password}
          {...form.register("password")}
        />
        <PasswordField
          label="Confirm password"
          autoComplete="new-password"
          error={form.formState.errors.confirmPassword}
          {...form.register("confirmPassword")}
        />
        {server?.inviteCodeRequired && (
          <Field
            label="Invite code"
            error={form.formState.errors.inviteCode}
            {...form.register("inviteCode")}
          />
        )}

        {create.error && <ErrorAlert error={create.error} />}

        <Button type="submit" color="primary" isLoading={create.isPending} fullWidth>
          Create account
        </Button>
      </form>

      <Divider />

      <div className="flex flex-col gap-2">
        <PdsSelect label="Create on" />
        <Alert tone="info">
          The server places your repository; your handle works across the whole
          namespace either way.
        </Alert>
      </div>
    </AuthCard>
  );
}
