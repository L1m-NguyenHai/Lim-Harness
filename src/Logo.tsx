// Connection mark: two open brackets around a shared core.
export default function Logo() {
  return (
    <svg
      className="app-logo"
      viewBox="0 0 32 32"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M13 5 5 10v12l8 5M19 5l8 5v12l-8 5"
        stroke="currentColor"
        strokeWidth="2.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <rect
        x="12.7"
        y="12.7"
        width="6.6"
        height="6.6"
        rx="1.2"
        transform="rotate(45 16 16)"
        fill="currentColor"
      />
    </svg>
  );
}
