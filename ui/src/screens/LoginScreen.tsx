import { useEffect, useState } from "react";
import { Button, Divider, Link } from "@heroui/react";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useMutation } from "@tanstack/react-query";
import { useAtom, useSetAtom } from "jotai";
import { IconKey, IconServer2, IconShieldLock } from "@tabler/icons-react";
import {
  assertionJson,
  beginPasskeyLogin,
  finishPasskeyLogin,
  passkeysAvailable,
  toPublicKeyRequest,
} from "../lib/security";
import { Field } from "../components/Field";
import { AuthCard } from "../components/AuthCard";
import { Alert, ErrorAlert } from "../components/Alert";
import { HandleField } from "../components/HandleField";
import { PasswordField } from "../components/PasswordField";
import { PdsSelect } from "../components/PdsSelect";
import { lastHandleAtom, pdsUrlAtom, sessionAtom } from "../atoms/store";
import { detectPds } from "../lib/resolve";
import { preferOrigin } from "../lib/servers";
import { createClient, XrpcError } from "../lib/xrpc";
import type { Session } from "../lib/types";

/// Sign-in is by handle only. A handle resolves to the PDS that hosts it, which
/// is what makes routing possible — an email address cannot be resolved, so it
/// would leave the app guessing which server to ask.
/// Both factors are reported as `AuthFactorTokenRequired`; only the status and
/// wording say which. An authenticator is a 401 naming `totpCode`, an emailed
/// code a 400.
function factorKind(error: unknown): "totp" | "email" | null {
  if (!(error instanceof XrpcError)) return null;
  if (error.error === "InvalidAuthFactorToken") return "totp";
  if (error.error !== "AuthFactorTokenRequired") return null;
  return /totp/i.test(error.message) || error.status === 401 ? "totp" : "email";
}

function factorField(kind: "totp" | "email" | null, value: string | undefined) {
  if (!value) return {};
  return kind === "totp" ? { totpCode: value } : { authFactorToken: value };
}

/// The handle to start with.
///
/// An OAuth client that already knows who is signing in sends `login_hint`, and
/// the PDS's own form honours it; the console must too, or the user retypes
/// their handle in the middle of a flow. A hint also beats the remembered
/// handle, because it is about this sign-in rather than the last one.
function hintedHandle(search: string, remembered: string): string {
  const params = new URLSearchParams(search);
  for (const key of ["login_hint", "handle", "identifier"]) {
    const value = params.get(key)?.trim();
    // An email cannot be resolved to a server, so it is not a usable hint here.
    if (value && !value.includes("@")) return value;
  }
  return remembered;
}

const makeSchema = (t: (key: string) => string) =>
  z.object({
    handle: z
      .string()
      .trim()
      .min(1, t("login.errorHandleRequired"))
      .refine((v) => !v.includes("@"), t("login.errorHandleEmail"))
      .refine(
        (v) => v.includes(".") || v.startsWith("did:"),
        t("login.errorHandleShape"),
      ),
    password: z.string().min(1, t("login.errorPasswordRequired")),
    authFactorToken: z.string().trim().optional(),
  });

type Values = z.infer<ReturnType<typeof makeSchema>>;

export function LoginScreen() {
  const { t } = useTranslation();
  const [pds, setPds] = useAtom(pdsUrlAtom);
  const [lastHandle, setLastHandle] = useAtom(lastHandleAtom);
  const setSession = useSetAtom(sessionAtom);
  const [detected, setDetected] = useState<string | null>(null);
  // Which second factor the server asked for. They are different fields: an
  // authenticator code goes in `totpCode`, an emailed code in
  // `authFactorToken`. Sending the wrong one never succeeds.
  const [needsFactor, setNeedsFactor] = useState<"totp" | "email" | null>(null);

  const form = useForm<Values>({
    resolver: zodResolver(makeSchema(t)),
    defaultValues: {
      handle: hintedHandle(window.location.search, lastHandle),
      password: "",
      authFactorToken: "",
    },
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

  /// Signing in with a passkey instead of a password. The handle still comes
  /// first: it is what locates the server to ask.
  const passkeyLogin = useMutation({
    mutationFn: async (values: Values) => {
      const resolution = await detectPds(values.handle);
      const base = preferOrigin(resolution.service);
      setPds(base);

      const client = createClient(base);
      const started = await beginPasskeyLogin(client, values.handle);

      const credential = (await navigator.credentials.get({
        publicKey: toPublicKeyRequest(started.publicKey),
      })) as PublicKeyCredential | null;
      if (!credential) throw new Error(t("login.passkeyNone"));

      // A user-verified passkey is already two factors — the device, and the
      // PIN or biometric that unlocked it — so no code is sent on top.
      return finishPasskeyLogin(client, {
        requestId: started.requestId,
        credential: assertionJson(credential),
      });
    },
    onSuccess: (session, values) => {
      setLastHandle(values.handle);
      setSession(session);
    },
  });

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
        ...factorField(needsFactor, values.authFactorToken),
      });
    },
    onSuccess: (session, values) => {
      setLastHandle(values.handle);
      setSession(session);
    },
    onError: (error) => {
      setNeedsFactor(factorKind(error) ?? needsFactor);
    },
  });

  return (
    <AuthCard
      title={t("login.title")}
      subtitle={t("login.subtitle")}
      footer={
        <>
          {t("login.noAccount")} <Link href="#/signup" size="sm">{t("login.createOne")}</Link>
          {" · "}
          <Link href="#/forgot" size="sm">{t("forgot.link")}</Link>
        </>
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={form.handleSubmit((values) => signIn.mutate(values))}
      >
        <HandleField
          label={t("common.handle")}
          placeholder={t("login.handlePlaceholder")}
          autoComplete="username"
          error={form.formState.errors.handle}
          {...form.register("handle")}
        />

        {detect.isPending && (
          <p className="text-xs text-foreground-500">{t("login.findingServer")}</p>
        )}
        {detected && !detect.isPending && (
          <div className="flex items-center gap-2 text-xs text-foreground-500">
            <IconServer2 size={14} />
            <span className="truncate">
              {t("login.signingInTo")} <span className="text-foreground">{new URL(detected).host}</span>
            </span>
          </div>
        )}

        <PasswordField
          label={t("common.password")}
          autoComplete="current-password"
          error={form.formState.errors.password}
          {...form.register("password")}
        />

        {needsFactor && (
          <Field
            label={t("login.factorLabel")}
            description={
              needsFactor === "totp"
                ? t("login.factorTotp")
                : t("login.factorEmail")
            }
            inputMode={needsFactor === "totp" ? "numeric" : undefined}
            autoComplete="one-time-code"
            startContent={
              <IconShieldLock size={16} className="shrink-0 text-default-400" aria-hidden />
            }
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
          {t("common.signIn")}
        </Button>
      </form>

      <Divider />

      {/* Other ways in. A passkey needs the handle too, because it is how the
          right server is found before any credential is offered. */}
      <div className="flex flex-col gap-2">
        <Button
          variant="bordered"
          startContent={<IconKey size={16} />}
          isDisabled={!passkeysAvailable() || !handle?.trim()}
          isLoading={passkeyLogin.isPending}
          onPress={() => passkeyLogin.mutate(form.getValues())}
          fullWidth
        >
          {t("login.passkey")}
        </Button>
        {!passkeysAvailable() && (
          <p className="text-xs text-foreground-500">
            {t("login.passkeyUnsupported")}
          </p>
        )}
        {passkeysAvailable() && !handle?.trim() && (
          <p className="text-xs text-foreground-500">
            {t("login.passkeyNeedsHandle")}
          </p>
        )}
        {passkeyLogin.error ? <ErrorAlert error={passkeyLogin.error} /> : null}
      </div>

      <Divider />

      <div className="flex flex-col gap-2">
        <PdsSelect label={t("login.serverLabel")} />
        {detect.error && (
          <Alert tone="info">
            {t("login.detectFailed")}
          </Alert>
        )}
      </div>
    </AuthCard>
  );
}
