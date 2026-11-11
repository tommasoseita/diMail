// Icone di diMail, disegnate qui: tratto da 1,75 su una griglia di 24 (specifica UX, sezione 3).
import type { SVGProps } from "react";

type P = SVGProps<SVGSVGElement> & { size?: number };

function Svg({ size = 18, children, ...rest }: P) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.75}
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...rest}>
      {children}
    </svg>
  );
}

export const Inbox = (p: P) => (
  <Svg {...p}>
    <path d="M4 13.5l2.4-7.2a1 1 0 0 1 .95-.68h9.3a1 1 0 0 1 .95.68L20 13.5V18a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1z" />
    <path d="M4 13.5h4.2l1.3 2.3h5l1.3-2.3H20" />
  </Svg>
);

export const Check = (p: P) => (
  <Svg {...p}>
    <path d="M5 12.6l4.3 4.3L19 7.2" />
  </Svg>
);

export const DoneAll = (p: P) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="8.2" />
    <path d="M8.3 12.3l2.5 2.5 4.9-5" />
  </Svg>
);

export const Send = (p: P) => (
  <Svg {...p}>
    <path d="M20 4L3.8 10.6l6.4 2.9 2.9 6.5z" />
    <path d="M20 4l-9.8 9.5" />
  </Svg>
);

export const Draft = (p: P) => (
  <Svg {...p}>
    <path d="M14 3.5H7.2a1.2 1.2 0 0 0-1.2 1.2v14.6a1.2 1.2 0 0 0 1.2 1.2h9.6a1.2 1.2 0 0 0 1.2-1.2V7.5z" />
    <path d="M14 3.5v4h4" />
    <path d="M9 15.5l4.5-4.5 1.5 1.5-4.5 4.5H9z" />
  </Svg>
);

export const Tag = (p: P) => (
  <Svg {...p}>
    <path d="M3.5 11.6V4.7a1.2 1.2 0 0 1 1.2-1.2h6.9l8.9 8.9-8.1 8.1z" />
    <circle cx="8.3" cy="8.3" r="1.4" />
  </Svg>
);

export const Trash = (p: P) => (
  <Svg {...p}>
    <path d="M4.5 7h15" />
    <path d="M9.5 7V4.5h5V7" />
    <path d="M6.8 7l.9 12.2a1 1 0 0 0 1 .9h6.6a1 1 0 0 0 1-.9L17.2 7" />
  </Svg>
);

export const Star = ({ filled, ...p }: P & { filled?: boolean }) => (
  <Svg {...p} fill={filled ? "currentColor" : "none"}>
    <path d="M12 4.2l2.3 4.8 5.2.7-3.8 3.6.9 5.2L12 16l-4.6 2.5.9-5.2-3.8-3.6 5.2-.7z" />
  </Svg>
);

export const Envelope = (p: P) => (
  <Svg {...p}>
    <rect x="3.5" y="5.5" width="17" height="13" rx="1.6" />
    <path d="M4 7l8 6 8-6" />
  </Svg>
);

export const Person = (p: P) => (
  <Svg {...p}>
    <circle cx="12" cy="8.3" r="3.8" />
    <path d="M4.8 19.6c1.3-3.6 4-5.6 7.2-5.6s5.9 2 7.2 5.6" />
  </Svg>
);

export const Help = (p: P) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="8.2" />
    <path d="M9.7 9.4a2.4 2.4 0 1 1 3.4 2.2c-.7.3-1.1.8-1.1 1.6v.4" />
    <path d="M12 16.7h.01" />
  </Svg>
);

export const ArrowLeft = (p: P) => (
  <Svg {...p}>
    <path d="M19 12H5.5" />
    <path d="M11 6.2L5.2 12l5.8 5.8" />
  </Svg>
);

export const Clip = (p: P) => (
  <Svg {...p}>
    <path d="M19.4 11.4l-7.9 7.9a4.8 4.8 0 0 1-6.8-6.8l8.2-8.2a3.2 3.2 0 0 1 4.5 4.5l-8.2 8.2a1.6 1.6 0 0 1-2.3-2.3l7.5-7.5" />
  </Svg>
);

export const Search = (p: P) => (
  <Svg {...p}>
    <circle cx="10.8" cy="10.8" r="6.3" />
    <path d="M15.6 15.6l4.4 4.4" />
  </Svg>
);

export const Stack = (p: P) => (
  <Svg {...p}>
    <path d="M12 4l8.5 4.5L12 13 3.5 8.5z" />
    <path d="M3.5 12.5L12 17l8.5-4.5" />
    <path d="M3.5 16.2L12 20.7l8.5-4.5" />
  </Svg>
);

export const PanelLeft = (p: P) => (
  <Svg {...p}>
    <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
    <path d="M9.5 4.5v15" />
  </Svg>
);

export const PanelRight = (p: P) => (
  <Svg {...p}>
    <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
    <path d="M14.5 4.5v15" />
  </Svg>
);

export const ChevronLeft = (p: P) => (
  <Svg {...p}>
    <path d="M14.5 6l-6 6 6 6" />
  </Svg>
);

export const ChevronRight = (p: P) => (
  <Svg {...p}>
    <path d="M9.5 6l6 6-6 6" />
  </Svg>
);

export const ChevronDown = (p: P) => (
  <Svg {...p}>
    <path d="M6 9.5l6 6 6-6" />
  </Svg>
);
export const Plus = (p: P) => (
  <Svg {...p}>
    <path d="M12 5v14M5 12h14" />
  </Svg>
);

export const Refresh = (p: P) => (
  <Svg {...p}>
    <path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3" />
    <path d="M19.8 4.6v3.6h-3.6" />
  </Svg>
);

export const Moon = (p: P) => (
  <Svg {...p}>
    <path d="M19.5 14.2A7.8 7.8 0 0 1 9.8 4.5a7.8 7.8 0 1 0 9.7 9.7z" />
  </Svg>
);

export const Sun = (p: P) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="3.8" />
    <path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4L7 17M17 7l1.4-1.4" />
  </Svg>
);

export const Bell = (p: P) => (
  <Svg {...p}>
    <path d="M6.5 16.5V11a5.5 5.5 0 0 1 11 0v5.5l1.5 1.5H5z" />
    <path d="M10 20.2a2.2 2.2 0 0 0 4 0" />
  </Svg>
);

export const Calendar = (p: P) => (
  <Svg {...p}>
    <rect x="4" y="5.5" width="16" height="14.5" rx="2.2" />
    <path d="M4 10h16M8.5 3.5v4M15.5 3.5v4" />
  </Svg>
);

export const Clock = (p: P) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="8.2" />
    <path d="M12 7.5V12l3 2" />
  </Svg>
);

export const Users = (p: P) => (
  <Svg {...p}>
    <circle cx="9.5" cy="8.5" r="3.3" />
    <path d="M3.8 19c.9-3.2 3-4.9 5.7-4.9s4.8 1.7 5.7 4.9" />
    <path d="M15.5 5.6a3.2 3.2 0 0 1 0 6M17.5 14.4c1.4.6 2.4 2 2.9 4.2" />
  </Svg>
);

export const Lines = (p: P) => (
  <Svg {...p}>
    <path d="M5 7h14M5 12h14M5 17h9" />
  </Svg>
);

export const FileIcon = (p: P) => (
  <Svg {...p}>
    <path d="M14 3.5H7.2a1.2 1.2 0 0 0-1.2 1.2v14.6a1.2 1.2 0 0 0 1.2 1.2h9.6a1.2 1.2 0 0 0 1.2-1.2V7.5z" />
    <path d="M14 3.5v4h4" />
  </Svg>
);

export const Mic = (p: P) => (
  <Svg {...p}>
    <rect x="9" y="3.5" width="6" height="11" rx="3" />
    <path d="M5.8 11.5a6.2 6.2 0 0 0 12.4 0M12 17.7v2.8" />
  </Svg>
);

export const At = (p: P) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="3.6" />
    <path d="M15.6 12v1.4a2.6 2.6 0 0 0 5.2 0V12a8.8 8.8 0 1 0-3.5 7" />
  </Svg>
);

export const Sparkle = (p: P) => (
  <Svg {...p}>
    <path d="M12 3.8l1.7 4.6a2 2 0 0 0 1.2 1.2l4.6 1.7-4.6 1.7a2 2 0 0 0-1.2 1.2L12 18.8l-1.7-4.6a2 2 0 0 0-1.2-1.2L4.5 11.3l4.6-1.7a2 2 0 0 0 1.2-1.2z" />
    <path d="M18.5 3.5v3M17 5h3" />
  </Svg>
);

export const Pencil = (p: P) => (
  <Svg {...p}>
    <path d="M4.5 19.5l1-4.2L15.8 5a1.9 1.9 0 0 1 2.7 0l.5.5a1.9 1.9 0 0 1 0 2.7L8.7 18.5z" />
    <path d="M13.8 7l3.2 3.2" />
  </Svg>
);

export const Alarm = (p: P) => (
  <Svg {...p}>
    <circle cx="12" cy="13" r="7" />
    <path d="M12 9.5V13l2.4 1.6M5 4.8L3.2 6.6M19 4.8l1.8 1.8" />
  </Svg>
);

export const Grip = (p: P) => (
  <Svg {...p}>
    <path d="M9 6.5h.01M15 6.5h.01M9 12h.01M15 12h.01M9 17.5h.01M15 17.5h.01" strokeWidth={2.6} />
  </Svg>
);

export const Dots = (p: P) => (
  <Svg {...p}>
    <path d="M6 12h.01M12 12h.01M18 12h.01" strokeWidth={2.6} />
  </Svg>
);

export const Archive = (p: P) => (
  <Svg {...p}>
    <rect x="3.8" y="4.5" width="16.4" height="4" rx="1" />
    <path d="M5.3 8.5V18a1 1 0 0 0 1 1h11.4a1 1 0 0 0 1-1V8.5M10 12.5h4" />
  </Svg>
);

export const Reply = (p: P) => (
  <Svg {...p}>
    <path d="M9.5 6.5L4.5 11.5l5 5" />
    <path d="M4.8 11.5H14a5.5 5.5 0 0 1 5.5 5.5v1" />
  </Svg>
);

export const ReplyAll = (p: P) => (
  <Svg {...p}>
    <path d="M11 6.5l-5 5 5 5M7 6.5l-5 5 5 5" />
    <path d="M6.3 11.5H14a5.5 5.5 0 0 1 5.5 5.5v1" />
  </Svg>
);

export const Forward = (p: P) => (
  <Svg {...p}>
    <path d="M14.5 6.5l5 5-5 5" />
    <path d="M19.2 11.5H10a5.5 5.5 0 0 0-5.5 5.5v1" />
  </Svg>
);

export const Close = (p: P) => (
  <Svg {...p}>
    <path d="M6.5 6.5l11 11M17.5 6.5l-11 11" />
  </Svg>
);

export const Eye = (p: P) => (
  <Svg {...p}>
    <path d="M2.8 12s3.3-6 9.2-6 9.2 6 9.2 6-3.3 6-9.2 6-9.2-6-9.2-6z" />
    <circle cx="12" cy="12" r="2.6" />
  </Svg>
);

export const Download = (p: P) => (
  <Svg {...p}>
    <path d="M12 4.5v10.5M7.5 10.8 12 15.3l4.5-4.5M5 19.5h14" />
  </Svg>
);

export const Expand = (p: P) => (
  <Svg {...p}>
    <path d="M14.5 4.5h5v5M19.5 4.5l-6 6M9.5 19.5h-5v-5M4.5 19.5l6-6" />
  </Svg>
);

export const Shrink = (p: P) => (
  <Svg {...p}>
    <path d="M19.5 9.5h-5v-5M14.5 9.5l6-6M4.5 14.5h5v5M9.5 14.5l-6 6" />
  </Svg>
);
