import { HeroUIProvider } from "@heroui/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { Provider as JotaiProvider, createStore } from "jotai";
import type { ReactElement } from "react";

/// A fresh Jotai store and query client per test, so state never leaks between
/// them.
export function renderApp(ui: ReactElement) {
  const store = createStore();
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });

  return {
    store,
    client,
    ...render(
      <JotaiProvider store={store}>
        <HeroUIProvider>
          <QueryClientProvider client={client}>{ui}</QueryClientProvider>
        </HeroUIProvider>
      </JotaiProvider>,
    ),
  };
}
