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
import { useLang } from "./lib/i18n";

/** Trocar o idioma remonta as telas: todo texto passa por t() na renderizacao. */
function App() {
  const lang = useLang();
  return <RouterProvider key={lang} router={router} />;
}

startLive();
startZoom();
startLinks();

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
}
