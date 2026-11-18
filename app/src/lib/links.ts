// Links externos (http/https) abrem no navegador padrao: o webview do Tauri ignora target=_blank.
import { commands } from "../bindings";

export function startLinks() {
  document.addEventListener("click", (e) => {
    if (e.defaultPrevented || e.button !== 0) return;
    const a = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (!a || !/^https?:\/\//i.test(a.href) || a.origin === window.location.origin) return;
    e.preventDefault();
    void commands.openUrl(a.href);
  });
}
