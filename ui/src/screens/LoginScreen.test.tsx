import { describe, expect, it } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LoginScreen } from "./LoginScreen";
import { renderApp } from "../test/render";
import { pdsUrlAtom, sessionAtom } from "../atoms/store";
import { BOB } from "../mocks/fixtures";

describe("signing in", () => {
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
