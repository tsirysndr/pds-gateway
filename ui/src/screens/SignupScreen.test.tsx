import { describe, expect, it } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SignupScreen } from "./SignupScreen";
import { renderApp } from "../test/render";
import { sessionAtom } from "../atoms/store";

async function fill(user: ReturnType<typeof userEvent.setup>, password: string, confirm: string) {
  await user.type(await screen.findByLabelText("Handle"), "newbie");
  await user.type(screen.getByLabelText("Email"), "newbie@example.com");
  await user.type(screen.getByLabelText("Password"), password);
  await user.type(screen.getByLabelText("Confirm password"), confirm);
  await user.click(screen.getByRole("button", { name: "Create account" }));
}

describe("creating an account", () => {
  it("refuses a mismatched confirmation without creating anything", async () => {
    const user = userEvent.setup();
    const { store } = renderApp(<SignupScreen />);

    await fill(user, "correct-horse", "correct-hose");

    // A typo in a password you cannot see would lock you out of a brand new
    // account, with nothing to recover from.
    expect(await screen.findByText(/Both passwords must match/i)).toBeInTheDocument();
    expect(store.get(sessionAtom)).toBeNull();
  });

  it("creates the account when both match", async () => {
    const user = userEvent.setup();
    const { store } = renderApp(<SignupScreen />);

    await fill(user, "correct-horse", "correct-horse");

    await waitFor(
      () => expect(store.get(sessionAtom)?.handle).toBeDefined(),
      { timeout: 5000 },
    );
  });

  it("still requires a long enough password", async () => {
    const user = userEvent.setup();
    renderApp(<SignupScreen />);

    await fill(user, "short", "short");

    expect(await screen.findByText(/at least 8 characters/i)).toBeInTheDocument();
  });
});
