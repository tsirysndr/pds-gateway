import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { Provider as JotaiProvider, createStore } from "jotai";
import { sessionAtom } from "../atoms/store";
import { handlers } from "../mocks/handlers";
import { ALICE, session } from "../mocks/fixtures";
import { AuthBackdrop } from "../components/AuthBackdrop";
import { AccountScreen } from "./AccountScreen";
import { AppPasswordsScreen } from "./AppPasswordsScreen";
import { InvitesScreen } from "./InvitesScreen";
import { LoginScreen } from "./LoginScreen";
import { PasskeysScreen } from "./PasskeysScreen";
import { SecurityScreen } from "./SecurityScreen";
import { SettingsScreen } from "./SettingsScreen";
import { SignupScreen } from "./SignupScreen";

/// Screens that need an account render inside a store that already has one.
function signedIn(Screen: React.ComponentType) {
  return function Wrapped() {
    const store = createStore();
    store.set(sessionAtom, session(ALICE));
    return (
      <JotaiProvider store={store}>
        <div className="mx-auto max-w-5xl p-4">
          <Screen />
        </div>
      </JotaiProvider>
    );
  };
}

const meta: Meta = { title: "Screens" };
export default meta;

export const SignIn: StoryObj = { render: () => <LoginScreen /> };

export const Backdrop: StoryObj = {
  name: "Sign in / the illustration alone",
  render: () => <AuthBackdrop />,
};

export const SignInUnresolvableHandle: StoryObj = {
  name: "Sign in / handle not found",
  render: () => <LoginScreen />,
  parameters: {
    msw: {
      handlers: [
        http.get("*/xrpc/com.atproto.identity.resolveHandle", () =>
          HttpResponse.json({ error: "UnableToResolveHandle" }, { status: 400 }),
        ),
        ...handlers,
      ],
    },
  },
};

export const SignUp: StoryObj = { render: () => <SignupScreen /> };
export const Account: StoryObj = { render: signedIn(AccountScreen) };
export const AppPasswords: StoryObj = { render: signedIn(AppPasswordsScreen) };
export const Invites: StoryObj = { render: signedIn(InvitesScreen) };
export const Settings: StoryObj = { render: signedIn(SettingsScreen) };

export const TwoFactorDisabled: StoryObj = {
  name: "Two-factor / off",
  render: signedIn(SecurityScreen),
};

export const TwoFactorEnrolling: StoryObj = {
  name: "Two-factor / scanning the QR code",
  render: signedIn(SecurityScreen),
  parameters: {
    msw: {
      handlers: [
        http.get("*/xrpc/social.rocksky.auth.getTwoFactor", () =>
          HttpResponse.json({ state: "pending" }),
        ),
        ...handlers,
      ],
    },
  },
};

export const TwoFactorEnabled: StoryObj = {
  name: "Two-factor / on",
  render: signedIn(SecurityScreen),
  parameters: {
    msw: {
      handlers: [
        http.get("*/xrpc/social.rocksky.auth.getTwoFactor", () =>
          HttpResponse.json({ state: "enabled", recoveryRemaining: 2 }),
        ),
        ...handlers,
      ],
    },
  },
};

export const PasskeysUnsupported: StoryObj = {
  name: "Passkeys / server has none",
  render: signedIn(PasskeysScreen),
  parameters: {
    msw: {
      handlers: [
        http.get("*/xrpc/social.rocksky.auth.listPasskeys", () =>
          HttpResponse.json(
            { error: "MethodNotImplemented", message: "Endpoint is not implemented" },
            { status: 501 },
          ),
        ),
        ...handlers,
      ],
    },
  },
};

export const TwoFactorUnsupported: StoryObj = {
  name: "Two-factor / server has none",
  render: signedIn(SecurityScreen),
  parameters: {
    msw: {
      handlers: [
        http.get("*/xrpc/social.rocksky.auth.getTwoFactor", () =>
          HttpResponse.json(
            { error: "MethodNotImplemented", message: "Endpoint is not implemented" },
            { status: 501 },
          ),
        ),
        ...handlers,
      ],
    },
  },
};

export const Passkeys: StoryObj = {
  render: signedIn(PasskeysScreen),
  parameters: {
    msw: {
      handlers: [
        http.get("*/xrpc/social.rocksky.auth.listPasskeys", () =>
          HttpResponse.json({
            passkeys: [
              { id: "cred-1", name: "MacBook", createdAt: "2026-02-01T10:00:00Z" },
            ],
          }),
        ),
        ...handlers,
      ],
    },
  },
};
