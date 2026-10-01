import { describe, expect, it } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LoginScreen } from "./LoginScreen";
import { renderApp } from "../test/render";
import { pdsUrlAtom, sessionAtom } from "../atoms/store";
import { BOB, CARA } from "../mocks/fixtures";

describe("signing in", () => {
  it("prefills the handle an OAuth client already knows", async () => {
    // Mid-flow, the client sends login_hint. Making the user retype their
    // handle there is the thing this avoids.
    window.history.replaceState(
      {},
      "",
      `/account/login?login_hint=${encodeURIComponent(BOB.handle)}`,
    );
    renderApp(<LoginScreen />);

    expect(await screen.findByLabelText("Handle")).toHaveValue(BOB.handle);

    // And the server is detected from it without any typing.
    expect(
      await screen.findByText(/Signing in to/i, {}, { timeout: 5000 }),
    ).toBeInTheDocument();

    window.history.replaceState({}, "", "/");
  });

  it("ignores an email as a hint, since it names no server", async () => {
    window.history.replaceState({}, "", "/account/login?login_hint=alice%40example.com");
    renderApp(<LoginScreen />);

    expect(await screen.findByLabelText("Handle")).toHaveValue("");

    window.history.replaceState({}, "", "/");
  });

  it("asks for a handle and refuses an email", async () => {
    const user = userEvent.setup();
    renderApp(<LoginScreen />);

    expect(screen.getByLabelText("Handle")).toBeInTheDocument();
    // There is no email field: an email cannot be resolved to a server.
    expect(screen.queryByLabelText(/email/i)).not.toBeInTheDocument();

    await user.type(screen.getByLabelText("Handle"), "alice@example.com");
    await user.type(screen.getByLabelText("Password"), "correct-horse");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(
      await screen.findByText(/handle, not your email/i),
    ).toBeInTheDocument();
  });

  it("detects the server that hosts the handle and signs in there", async () => {
    const user = userEvent.setup();
    const { store } = renderApp(<LoginScreen />);

    await user.type(screen.getByLabelText("Handle"), BOB.handle);

    // The detection banner, not the dropdown, must name the resolved host.
    expect(
      await screen.findByText(/Signing in to/i, {}, { timeout: 3000 }),
    ).toBeInTheDocument();
    await waitFor(() => expect(store.get(pdsUrlAtom)).toBe(BOB.service), {
      timeout: 3000,
    });

    await user.type(screen.getByLabelText("Password"), "correct-horse");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    await waitFor(() => expect(store.get(sessionAtom)?.handle).toBe(BOB.handle), {
      timeout: 5000,
    });
  });

  it("sends an authenticator code in the field the server asks for", async () => {
    const user = userEvent.setup();
    const { store } = renderApp(<LoginScreen />);

    await user.type(screen.getByLabelText("Handle"), CARA.handle);
    await screen.findByText(/Signing in to/i, {}, { timeout: 3000 });
    await user.type(screen.getByLabelText("Password"), "correct-horse");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    // The server asked for a second factor, so the field appears.
    const code = await screen.findByLabelText("Code", {}, { timeout: 5000 });
    expect(screen.getByText(/From your authenticator app/i)).toBeInTheDocument();

    await user.type(code, "123456");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    // Only succeeds if the code went in `totpCode`; `authFactorToken` is the
    // emailed-code field and the server ignores it here.
    await waitFor(() => expect(store.get(sessionAtom)?.handle).toBe(CARA.handle), {
      timeout: 5000,
    });
  });

  it("offers a passkey as another way in, once a handle is given", async () => {
    // happy-dom has no WebAuthn, so stand in an authenticator before render:
    // the button is hidden entirely when the browser cannot do passkeys.
    Object.defineProperty(window, "PublicKeyCredential", {
      configurable: true,
      value: function PublicKeyCredential() {},
    });
    Object.defineProperty(navigator, "credentials", {
      configurable: true,
      value: {
        get: async () => ({
          id: "cred-1",
          rawId: new Uint8Array([1, 2, 3]).buffer,
          type: "public-key",
          response: {
            clientDataJSON: new Uint8Array([4]).buffer,
            authenticatorData: new Uint8Array([5]).buffer,
            signature: new Uint8Array([6]).buffer,
            userHandle: null,
          },
        }),
      },
    });

    const user = userEvent.setup();
    const { store } = renderApp(<LoginScreen />);

    const button = screen.getByRole("button", { name: /Sign in with a passkey/i });

    // The handle locates the server, so it is needed before any credential.
    expect(button).toBeDisabled();
    expect(screen.getByText(/Enter your handle first/i)).toBeInTheDocument();

    await user.type(screen.getByLabelText("Handle"), BOB.handle);
    await waitFor(() => expect(button).toBeEnabled());

    await user.click(button);

    await waitFor(() => expect(store.get(sessionAtom)?.handle).toBe(BOB.handle), {
      timeout: 5000,
    });
  });

  it("shows the server's own message when the password is wrong", async () => {
    const user = userEvent.setup();
    renderApp(<LoginScreen />);

    await user.type(screen.getByLabelText("Handle"), BOB.handle);
    await user.type(screen.getByLabelText("Password"), "wrong");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(
      await screen.findByText(/Invalid identifier or password/i, {}, { timeout: 5000 }),
    ).toBeInTheDocument();
  });
});
