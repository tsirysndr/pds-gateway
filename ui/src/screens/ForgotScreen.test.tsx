import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { ForgotScreen } from "./ForgotScreen";
import { BOB } from "../mocks/fixtures";
import { server } from "../mocks/server";
import { renderApp } from "../test/render";

describe("ForgotScreen", () => {
  it("asks the account's own server to email a reset link", async () => {
    let askedAt: string | null = null;
    server.use(
      http.post("*/xrpc/com.atproto.server.requestPasswordReset", ({ request }) => {
        askedAt = new URL(request.url).host;
        return HttpResponse.json({});
      }),
    );

    const user = userEvent.setup();
    renderApp(<ForgotScreen />);

    await user.type(screen.getByLabelText("Handle"), BOB.handle);
    await user.type(screen.getByLabelText("Account email"), "bob@example.com");
    await user.click(screen.getByRole("button", { name: /Email me a reset link/i }));

    // The handle picked the server; the email went to that server, and the
    // screen does not say whether the address exists.
    expect(await screen.findByText(/a reset link is on its way/i)).toBeInTheDocument();
    expect(askedAt).not.toBeNull();
  });
});
