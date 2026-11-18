import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import "./styles.css";
import { queryClient } from "./lib/queries";
import { startLive } from "./lib/live";
import { router } from "./router";
import { startZoom } from "./lib/zoom";
import { startLinks } from "./lib/links";

startLive();
startZoom();
startLinks();

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </StrictMode>,
  );
}
