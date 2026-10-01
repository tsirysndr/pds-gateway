import { useEffect, useState } from "react";
import { HeroUIProvider } from "@heroui/react";
import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { useAtomValue } from "jotai";
import { isSignedInAtom, languageAtom } from "./atoms/store";
import { detectLanguage, setupI18n } from "./i18n";
import { Layout } from "./components/Layout";
import { AccountScreen } from "./screens/AccountScreen";
import { AppPasswordsScreen } from "./screens/AppPasswordsScreen";
import { InvitesScreen } from "./screens/InvitesScreen";
import { LoginScreen } from "./screens/LoginScreen";
import { PasskeysScreen } from "./screens/PasskeysScreen";
import { RepoScreen } from "./screens/RepoScreen";
import { SecurityScreen } from "./screens/SecurityScreen";
import { SettingsScreen } from "./screens/SettingsScreen";
import { SignupScreen } from "./screens/SignupScreen";

/// The screen the server mounted this page at.
///
/// The console answers some of the PDS's own paths, so arriving at
/// `/account/login` must show sign-in without a hash. The hash still wins once
/// the user navigates, which keeps routing to one mechanism.
function routeFromPath(pathname: string): string | null {
  const path = pathname.replace(/\/+$/, "") || "/";
  if (path.endsWith("/account/login")) return "login";
  if (path.endsWith("/account/signup")) return "signup";
  if (path === "/") return "root";
  return null;
}

/// Hash routing, so the app works wherever the server mounts it without
/// needing a history rewrite rule.
function useHashRoute(fallback: string) {
  const initial = () =>
    window.location.hash.replace(/^#\/?/, "") ||
    routeFromPath(window.location.pathname) ||
    fallback;

  const [route, setRoute] = useState(initial);

  useEffect(() => {
    const onChange = () => setRoute(initial());
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fallback]);

  const navigate = (next: string) => {
    window.location.hash = `#/${next}`;
    setRoute(next);
  };

  return [route, navigate] as const;
}

const SIGNED_IN: Record<string, React.ComponentType> = {
  account: AccountScreen,
  repo: RepoScreen,
  sessions: AppPasswordsScreen,
  security: SecurityScreen,
  passkeys: PasskeysScreen,
  invites: InvitesScreen,
  settings: SettingsScreen,
};

function SignedInRoute({
  route,
  onNavigate,
}: {
  route: string;
  onNavigate: (next: string) => void;
}) {
  const known = route in SIGNED_IN ? route : "account";
  const Screen = SIGNED_IN[known] ?? AccountScreen;
  return (
    <Layout route={known} onNavigate={onNavigate}>
      <Screen />
    </Layout>
  );
}

/// One i18n instance for the page, started before anything renders text.
function useLanguageSetup() {
  const stored = useAtomValue(languageAtom);
  const language = detectLanguage(stored || null, navigator.languages ?? []);
  const i18n = setupI18n(language);
  useEffect(() => {
    if (i18n.language !== language) void i18n.changeLanguage(language);
  }, [i18n, language]);
}

export function App({ client }: { client: QueryClient }) {
  useLanguageSetup();
  const signedIn = useAtomValue(isSignedInAtom);
  const [route, navigate] = useHashRoute(signedIn ? "account" : "login");

  // "root" means the page was served at "/": show the account when there is a
  // session, and sign-in otherwise.
  const resolved = route === "root" ? (signedIn ? "account" : "login") : route;

  return (
    <HeroUIProvider>
      <QueryClientProvider client={client}>
        {resolved === "signup" ? (
          <SignupScreen />
        ) : signedIn ? (
          <SignedInRoute route={resolved} onNavigate={navigate} />
        ) : (
          <LoginScreen />
        )}
      </QueryClientProvider>
    </HeroUIProvider>
  );
}
