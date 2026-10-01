import { definePreview } from "@storybook/react-vite";
import addonMsw from "msw-storybook-addon";
import { HeroUIProvider } from "@heroui/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Provider as JotaiProvider, createStore } from "jotai";
import { handlers } from "../src/mocks/handlers";
import "../src/styles.css";

export default definePreview({
  addons: [addonMsw()],
  parameters: {
    layout: "fullscreen",
    msw: { handlers },
    backgrounds: { disable: true },
  },
  decorators: [
    (Story) => {
      // A store and query client per story, so one story's session never leaks
      // into the next.
      const store = createStore();
      const client = new QueryClient({
        defaultOptions: { queries: { retry: false } },
      });
      return (
        <JotaiProvider store={store}>
          <HeroUIProvider>
            <QueryClientProvider client={client}>
              <div className="dark min-h-svh bg-background text-foreground">
                <Story />
              </div>
            </QueryClientProvider>
          </HeroUIProvider>
        </JotaiProvider>
      );
    },
  ],
});
