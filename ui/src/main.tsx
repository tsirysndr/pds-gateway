import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient } from "@tanstack/react-query";
import "./styles.css";
import { App } from "./App";

const client = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, refetchOnWindowFocus: false, staleTime: 15_000 },
  },
});

const container = document.getElementById("root");
if (container) {
  createRoot(container).render(
    <StrictMode>
      <App client={client} />
    </StrictMode>,
  );
}
