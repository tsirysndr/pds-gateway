import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import { AuthCard } from "./AuthCard";
import { renderApp } from "../test/render";

describe("the auth backdrop", () => {
  it("is drawn, and not at a negative z-index", () => {
    const { container } = renderApp(
      <AuthCard title="Sign in">
        <p>form</p>
      </AuthCard>,
    );

    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();

    // A child painted at a negative z-index goes behind its parent's
    // background, and `body` has an opaque one. That hid the drawing entirely.
    const backdrop = svg!.parentElement!;
    expect(backdrop.className).not.toMatch(/-z-\d/);
    expect(backdrop.className).toMatch(/\bz-0\b/);

    // The scene itself, not just an empty canvas.
    expect(svg!.querySelectorAll("path").length).toBeGreaterThan(5);
    expect(svg!.querySelector("rect")).not.toBeNull();
  });

  it("stays out of the way of the form", () => {
    renderApp(
      <AuthCard title="Sign in">
        <p>form</p>
      </AuthCard>,
    );

    // Decorative: no role, no label, and it cannot take a click.
    const backdrop = document.querySelector("[aria-hidden='true']");
    expect(backdrop?.className).toMatch(/pointer-events-none/);

    expect(screen.getByText("form")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Sign in" })).toBeInTheDocument();
  });
});
