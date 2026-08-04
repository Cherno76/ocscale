import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

const style = document.createElement("style");
style.textContent = `
  html, body, #root { margin: 0; padding: 0; height: 100%; background: transparent; }
  * { box-sizing: border-box; }
  /* Let form controls / scrollbars follow the active theme (light vs dark). */
  :root { color-scheme: light dark; }
  /* Popover entrance: fades in and settles from just above the tray anchor. */
  @keyframes om-pop-in {
    from { opacity: 0; transform: translateY(8px) scale(0.985); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }
  /* Content cross-fade used on period / tab switches. */
  @keyframes om-fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }
  .om-fade-in { animation: om-fade-in 0.24s ease-out; }
  /* Theme-aware hover well for small toolbar-style buttons (uses --om-hover
     set on the panel root by the active theme). */
  .om-iconbtn { transition: background .15s, color .15s; }
  .om-iconbtn:hover { background: var(--om-hover); }
  /* Respect the OS "reduce motion" setting app-wide. */
  @media (prefers-reduced-motion: reduce) {
    *, *::before, *::after {
      animation-duration: 0.01ms !important;
      animation-iteration-count: 1 !important;
      transition-duration: 0.01ms !important;
    }
  }
  .om-scroll { scrollbar-width: none; -ms-overflow-style: none; }
  .om-scroll::-webkit-scrollbar { width: 0; height: 0; display: none; }
  /* Hidden scrollbars for the capped inner lists (model/agent/session rows):
     same invisible-scrollbar treatment as the panel body. */
  .om-nobar { scrollbar-width: none; -ms-overflow-style: none; }
  .om-nobar::-webkit-scrollbar { width: 0; height: 0; display: none; }
  /* During a theme flip we add this class for a couple of frames so the whole
     panel repaints in the new theme in one step. Without it, every element's
     CSS transition (e.g. the Segmented selected pill's "background .15s")
     cross-fades the old color into the new one — most visible as the white
     selected pill fading on a background light→dark switch made while hidden. */
  .ts-no-transition, .ts-no-transition * { transition: none !important; }
`;
document.head.appendChild(style);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
