export default function OfflineIndicator() {
  return (
    <div
      className="flex items-center gap-2 px-3 py-1.5 text-xs"
      style={{ backgroundColor: "rgba(245, 158, 11, 0.1)", color: "#f59e0b" }}
    >
      <span
        className="inline-block w-2 h-2 rounded-full"
        style={{
          backgroundColor: "#f59e0b",
          animation: "pulse 2s infinite",
        }}
      />
      Offline &mdash; changes will sync when reconnected
      <style>{`
        @keyframes pulse {
          0%, 100% { opacity: 1; }
          50% { opacity: 0.4; }
        }
      `}</style>
    </div>
  );
}
