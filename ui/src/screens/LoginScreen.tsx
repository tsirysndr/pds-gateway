import { useEffect, useState } from "react";
import { Button, Divider, Link } from "@heroui/react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useMutation } from "@tanstack/react-query";
import { useAtom, useSetAtom } from "jotai";
import { IconServer2 } from "@tabler/icons-react";
import { AuthCard } from "../components/AuthCard";
import { Alert, ErrorAlert } from "../components/Alert";
import { Field } from "../components/Field";
import { PdsSelect } from "../components/PdsSelect";
import { lastHandleAtom, pdsUrlAtom, sessionAtom } from "../atoms/store";
import { detectPds } from "../lib/resolve";
import { preferOrigin } from "../lib/servers";
import { createClient } from "../lib/xrpc";
import type { Session } from "../lib/types";

/// Sign-in is by handle only. A handle resolves to the PDS that hosts it, which
/// is what makes routing possible — an email address cannot be resolved, so it
/// would leave the app guessing which server to ask.
const schema = z.object({
  handle: z
    .string()
    .trim()
    .min(1, "Enter your handle")
    .refine((v) => !v.includes("@"), "Sign in with your handle, not your email")
    .refine(
      (v) => v.includes(".") || v.startsWith("did:"),
      "Handles look like alice.example.com",
    ),
  password: z.string().min(1, "Enter your password"),
  authFactorToken: z.string().trim().optional(),
});

type Values = z.infer<typeof schema>;

export function LoginScreen() {
  const [pds, setPds] = useAtom(pdsUrlAtom);
  const [lastHandle, setLastHandle] = useAtom(lastHandleAtom);
  const setSession = useSetAtom(sessionAtom);
  const [detected, setDetected] = useState<string | null>(null);
  const [needsFactor, setNeedsFactor] = useState(false);

  const form = useForm<Values>({
    resolver: zodResolver(schema),
    defaultValues: { handle: lastHandle, password: "", authFactorToken: "" },
  });

  const handle = form.watch("handle");

  // Find the PDS as soon as the handle looks complete, so signing in goes
  // straight to the right server instead of failing on the wrong one.
  const detect = useMutation({
    mutationFn: (value: string) => detectPds(value),
    onSuccess: (resolution) => {
      const target = preferOrigin(resolution.service);
      setPds(target);
      setDetected(target);
    },
  });

  useEffect(() => {
    const value = handle?.trim() ?? "";
    if (value.length < 3 || value.includes("@")) {
      setDetected(null);
      return;
    }
    const timer = setTimeout(() => detect.mutate(value), 450);
    return () => clearTimeout(timer);
    // `detect` is a stable mutation handle from react-query.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [handle]);

  const signIn = useMutation({
    mutationFn: async (values: Values) => {
      // Detection usually landed while typing. Resolve only if it has not, so a
      // fast submit still reaches the right server without paying for a second
      // lookup every time.
      let base = detected ?? pds;
      if (!detected) {
        try {
          const resolution = await detectPds(values.handle);
          base = preferOrigin(resolution.service);
          setPds(base);
        } catch {
          // Fall back to the selected server; it may know a handle we cannot
          // resolve from here.
        }
      }

      return createClient(base).post<Session>("com.atproto.server.createSession", {
        identifier: values.handle,
        password: values.password,
        ...(values.authFactorToken ? { authFactorToken: values.authFactorToken } : {}),
      });
    },
    onSuccess: (session, values) => {
      setLastHandle(values.handle);
      setSession(session);
    },
    onError: (error) => {
      const name = (error as { error?: string }).error;
      if (name === "AuthFactorTokenRequired") setNeedsFactor(true);
    },
  });

  return (
    <AuthCard
      title="Sign in"
      subtitle="Use the handle for your account."
      footer={
        <>
          No account? <Link href="#/signup" size="sm">Create one</Link>
        </>
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={form.handleSubmit((values) => signIn.mutate(values))}
      >
        <Field
          label="Handle"
          placeholder="alice.rocksky.social"
          autoComplete="username"
          autoCapitalize="none"
          spellCheck="false"
          error={form.formState.errors.handle}
          {...form.register("handle")}
        />

        {detect.isPending && (
          <p className="text-xs text-foreground-500">Finding your server…</p>
        )}
        {detected && !detect.isPending && (
          <div className="flex items-center gap-2 text-xs text-foreground-500">
            <IconServer2 size={14} />
            <span className="truncate">
              Signing in to <span className="text-foreground">{new URL(detected).host}</span>
            </span>
          </div>
        )}

        <Field
          label="Password"
          type="password"
          autoComplete="current-password"
          error={form.formState.errors.password}
          {...form.register("password")}
        />

        {needsFactor && (
          <Field
            label="Confirmation code"
            description="Sent to the email on your account."
            autoComplete="one-time-code"
            error={form.formState.errors.authFactorToken}
            {...form.register("authFactorToken")}
          />
        )}

        {signIn.error && <ErrorAlert error={signIn.error} />}

        <Button
          type="submit"
          color="primary"
          isLoading={signIn.isPending}
          fullWidth
        >
          Sign in
        </Button>
      </form>

      <Divider />

      <div className="flex flex-col gap-2">
        <PdsSelect label="Server (detected from your handle)" />
        {detect.error && (
          <Alert tone="info">
            Could not detect a server for that handle yet. Pick one above if you
            know it.
          </Alert>
        )}
      </div>
    </AuthCard>
  );
}
