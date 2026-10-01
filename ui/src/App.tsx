import { useEffect, useState } from "react";
import { HeroUIProvider } from "@heroui/react";
import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { useAtomValue } from "jotai";
import { isSignedInAtom } from "./atoms/store";
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

/// Hash routing, so the app works wherever the server mounts it without
/// needing a history rewrite rule.
function useHashRoute(fallback: string) {
  const [route, setRoute] = useState(
    () => window.location.hash.replace(/^#\/?/, "") || fallback,
  );

  useEffect(() => {
    const onChange = () =>
      setRoute(window.location.hash.replace(/^#\/?/, "") || fallback);
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
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

export function App({ client }: { client: QueryClient }) {
  const signedIn = useAtomValue(isSignedInAtom);
  const [route, navigate] = useHashRoute(signedIn ? "account" : "login");

  return (
    <HeroUIProvider>
      <QueryClientProvider client={client}>
        {signedIn ? (
          <SignedInRoute route={route} onNavigate={navigate} />
        ) : route === "signup" ? (
          <SignupScreen />
        ) : (
          <LoginScreen />
        )}
      </QueryClientProvider>
    </HeroUIProvider>
  );
}
