// Icones de traco (16px), herdam a cor do texto.
import type { SVGProps } from "react";

type P = SVGProps<SVGSVGElement> & { size?: number };
const base = (size = 16, sw = 1.5) =>
  ({ width: size, height: size, viewBox: "0 0 16 16", fill: "none", stroke: "currentColor", strokeWidth: sw, "aria-hidden": true }) as const;

export const IChannels = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><rect x="2" y="2.5" width="12" height="3" rx="1" /><rect x="2" y="6.5" width="12" height="3" rx="1" /><rect x="2" y="10.5" width="12" height="3" rx="1" /></svg>
);
export const ILive = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><circle cx="8" cy="8" r="2" /><path d="M4.5 4.5a5 5 0 0 0 0 7M11.5 4.5a5 5 0 0 1 0 7" /></svg>
);
export const IAgents = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><path d="M3 4h6M3 8h10M3 12h4" /><circle cx="11" cy="4" r="1.6" /><circle cx="9" cy="12" r="1.6" /></svg>
);
export const IUsage = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><path d="M2.5 13.5h11" /><rect x="3.5" y="8" width="2.2" height="4" rx=".6" /><rect x="7" y="4.5" width="2.2" height="7.5" rx=".6" /><rect x="10.5" y="6.5" width="2.2" height="5.5" rx=".6" /></svg>
);
export const IUser = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><circle cx="8" cy="5.5" r="2.5" /><path d="M3 13.5c.6-2.4 2.6-3.8 5-3.8s4.4 1.4 5 3.8" /></svg>
);
export const IPlus = ({ size, ...p }: P) => (
  <svg {...base(size, 1.6)} {...p}><path d="M8 3v10M3 8h10" /></svg>
);
export const IChevron = ({ size, ...p }: P) => (
  <svg {...base(size, 1.6)} {...p} className={`rtl:-scale-x-100 ${p.className ?? ""}`}><path d="m6 3 5 5-5 5" /></svg>
);
export const IBack = ({ size, ...p }: P) => (
  <svg {...base(size, 1.6)} {...p} className={`rtl:-scale-x-100 ${p.className ?? ""}`}><path d="m10 3-5 5 5 5" /></svg>
);
export const IX = ({ size, ...p }: P) => (
  <svg {...base(size, 1.6)} {...p}><path d="m4 4 8 8M12 4l-8 8" /></svg>
);
export const ICheck = ({ size, ...p }: P) => (
  <svg {...base(size, 1.8)} {...p}><path d="m3.5 8.5 3 3 6-7" /></svg>
);
export const IPause = ({ size, ...p }: P) => (
  <svg {...base(size, 1.6)} {...p}><path d="M5.5 3.5v9M10.5 3.5v9" /></svg>
);
export const IPlay = ({ size, ...p }: P) => (
  <svg {...base(size, 1.6)} {...p}><path d="M5 3.5v9l7-4.5z" /></svg>
);
export const ISpin = ({ size, ...p }: P) => (
  <svg {...base(size, 2)} {...p} className={`anim-spin ${p.className ?? ""}`}><path d="M8 2a6 6 0 1 1-6 6" strokeLinecap="round" /></svg>
);
export const INote = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><path d="M4 2.5h6l2.5 2.5v8.5H4z" /><path d="M6 7.5h4M6 10h3" /></svg>
);
export const IOnce = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><circle cx="8" cy="8" r="5.5" /><path d="M8 5v3l2 1.5" /></svg>
);
export const IShield = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><path d="M8 1.8 2.8 4v3.6c0 3.1 2.2 5.6 5.2 6.6 3-1 5.2-3.5 5.2-6.6V4z" /></svg>
);
export const ILock = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><rect x="3" y="7" width="10" height="7" rx="1.5" /><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" /></svg>
);
export const IGlobe = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><circle cx="8" cy="8" r="5.5" /><path d="M2.5 8h11M8 2.5c1.6 1.6 2.4 3.4 2.4 5.5S9.6 11.9 8 13.5C6.4 11.9 5.6 10.1 5.6 8S6.4 4.1 8 2.5z" /></svg>
);
export const ISpark = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><path d="M8 2v3M8 11v3M2 8h3M11 8h3M4 4l2 2M10 10l2 2M12 4l-2 2M6 10l-2 2" /></svg>
);
export const ITikTok = ({ size = 20, ...p }: P) => (
  <svg width={size} height={size} viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={1.6} aria-hidden {...p}><path d="M11.5 3v9.5a2.8 2.8 0 1 1-2.8-2.8" /><path d="M11.5 3c.4 2.2 1.9 3.6 4 3.8" /></svg>
);
export const IYouTube = ({ size = 20, ...p }: P) => (
  <svg width={size} height={size} viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={1.6} aria-hidden {...p}><rect x="2" y="4.5" width="16" height="11" rx="3" /><path d="M8.5 7.8v4.4l3.8-2.2z" fill="currentColor" /></svg>
);
export const IMoon = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><path d="M12.5 10A5 5 0 0 1 6 3.5a5 5 0 1 0 6.5 6.5z" /></svg>
);
export const ISun = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><circle cx="8" cy="8" r="2.8" /><path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1 1M11.6 11.6l1 1M12.6 3.4l-1 1M4.4 11.6l-1 1" /></svg>
);
export const ISunrise = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><path d="M2 12.5h12M4.5 12.5a3.5 3.5 0 0 1 7 0M8 3v3M5 6.5l1 1M11 6.5l-1 1" /></svg>
);
export const ISunset = ({ size, ...p }: P) => (
  <svg {...base(size)} {...p}><path d="M2 12.5h12M4.5 12.5a3.5 3.5 0 0 1 7 0M8 6V3M6.5 4.5 8 6l1.5-1.5" /></svg>
);
