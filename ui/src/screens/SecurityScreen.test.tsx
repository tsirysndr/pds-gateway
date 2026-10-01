import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SecurityScreen } from "./SecurityScreen";
import { renderApp } from "../test/render";
import { server } from "../mocks/server";
import { sessionAtom } from "../atoms/store";
import { ALICE, session } from "../mocks/fixtures";

function signedIn() {
  const rendered = renderApp(<SecurityScreen />);
  rendered.store.set(sessionAtom, session(ALICE));
  return rendered;
}

describe("two-factor over social.rocksky.auth", () => {
  it("enrols, showing the QR code and the secret to type", async () => {
    const user = userEvent.setup();
    signedIn();

    await user.type(await screen.findByLabelText("Password"), "correct-horse");
    await user.click(screen.getByRole("button", { name: "Set up" }));

    // The otpauth URI becomes the QR code, and the secret is offered for
    // authenticators that cannot scan.
    expect(await screen.findByText(/Scan this with your authenticator/i)).toBeInTheDocument();
    expect(await screen.findByText("JBSWY3DPEHPK3PXP")).toBeInTheDocument();
  });

  it("reports the server's own message when the password is wrong", async () => {
    const user = userEvent.setup();
    signedIn();

    await user.type(await screen.findByLabelText("Password"), "wrong");
    await user.click(screen.getByRole("button", { name: "Set up" }));

    expect(await screen.findByText(/Incorrect password/i)).toBeInTheDocument();
  });

  it("says so plainly when the server does not implement the methods", async () => {
    // A PDS without these endpoints answers MethodNotImplemented, which is how
    // the console tells "not offered" from "refused".
    server.use(
      http.get("*/xrpc/social.rocksky.auth.getTwoFactor", () =>
        HttpResponse.json(
          { error: "MethodNotImplemented", message: "Endpoint is not implemented" },
          { status: 501 },
        ),
      ),
    );
    signedIn();

    expect(
      await screen.findByText(/does not implement/i, {}, { timeout: 5000 }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Set up" })).not.toBeInTheDocument();
  });

  it("offers a way out when enrolment is pending and the QR code is gone", async () => {
    // Reloading the page loses the secret: it is held in memory because the
    // server never returns it again. Without a way to restart, the account is
    // stuck at "pending" forever.
    server.use(
      http.get("*/xrpc/social.rocksky.auth.getTwoFactor", () =>
        HttpResponse.json({ state: "pending", recoveryRemaining: 0 }),
      ),
    );
    const user = userEvent.setup();
    signedIn();

    expect(
      await screen.findByText(/cannot be displayed a second time/i, {}, { timeout: 5000 }),
    ).toBeInTheDocument();

    // Confirming is still possible for someone who did scan it.
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();

    // And a fresh QR code can be obtained.
    const restart = screen.getByRole("button", { name: "Get a new QR code" });
    const form = within(restart.closest("form")!);
    await user.type(form.getByLabelText("Password"), "correct-horse");
    await user.click(restart);

    expect(
      await screen.findByText("JBSWY3DPEHPK3PXP", {}, { timeout: 5000 }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Scan this with your authenticator/i)).toBeInTheDocument();
  });

  it("reports a wrong code as a bad code", async () => {
    const user = userEvent.setup();
    signedIn();

    await user.type(await screen.findByLabelText("Password"), "correct-horse");
    await user.click(screen.getByRole("button", { name: "Set up" }));
    await screen.findByText(/Scan this with your authenticator/i);

    await user.type(screen.getByLabelText("Code"), "000000");
    await user.click(screen.getByRole("button", { name: "Confirm" }));

    // The server's own message, not a generic failure.
    expect(
      await screen.findByText(/That code is not valid/i, {}, { timeout: 5000 }),
    ).toBeInTheDocument();
  });

  it("shows recovery codes once two-factor is on", async () => {
    server.use(
      http.get("*/xrpc/social.rocksky.auth.getTwoFactor", () =>
        HttpResponse.json({ state: "enabled", recoveryRemaining: 2 }),
      ),
    );
    const user = userEvent.setup();
    signedIn();

    // Enabled renders two identical password/code pairs — recovery and disable —
    // so scope to the form that owns this button.
    const submit = await screen.findByRole("button", { name: "Generate new codes" });
    const form = within(submit.closest("form")!);
    await user.type(form.getByLabelText("Password"), "correct-horse");
    await user.type(form.getByLabelText("Code"), "123456");
    await user.click(submit);

    await waitFor(() => expect(screen.getByText("EEEE-FFFF")).toBeInTheDocument(), {
      timeout: 5000,
    });
  });
});
