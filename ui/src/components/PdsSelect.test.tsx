import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PdsSelect } from "./PdsSelect";
import { renderApp } from "../test/render";

describe("the server picker", () => {
  it("lists domains and nothing else", async () => {
    const user = userEvent.setup();
    renderApp(<PdsSelect />);

    await user.click(await screen.findByRole("button"));

    const options = await screen.findAllByRole("option");
    const labels = options.map((option) => option.textContent?.trim() ?? "");

    expect(labels).toContain("localhost:3000");
    expect(labels).toContain("radxa.rocksky.social");

    // No node names, and nothing appended to the domain.
    for (const label of labels) {
      expect(label).not.toMatch(/·/);
      expect(label).not.toMatch(/\b(local|radxa|raspberrypi4|detected)\b\s*$/);
      expect(label).toMatch(/^[a-z0-9.:-]+$/);
    }
  });
});
