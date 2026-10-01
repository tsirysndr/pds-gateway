import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import i18n from "../i18n";
import { LoginScreen } from "../screens/LoginScreen";
import { renderApp } from "../test/render";

describe("the language dropdown", () => {
  it("switches the whole screen, and French really is French", async () => {
    const user = userEvent.setup();
    renderApp(<LoginScreen />);

    expect(screen.getByRole("heading", { name: "Sign in" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Language/i }));
    await user.click(await screen.findByRole("option", { name: "Français" }));

    expect(await screen.findByRole("heading", { name: "Connexion" })).toBeInTheDocument();
    expect(screen.getByLabelText("Identifiant")).toBeInTheDocument();

    // Back, so the other tests keep asserting English.
    await i18n.changeLanguage("en");
  });
});
