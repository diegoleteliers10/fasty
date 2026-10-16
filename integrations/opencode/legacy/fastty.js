import { spawn } from "node:child_process";

const publish = (state, kind) => {
  const args = ["agent-status", state, "opencode"];
  if (kind) args.push(kind);
  const child = spawn("fastty", args, { stdio: "ignore" });
  child.on("error", () => {});
};

export const stateForEvent = (event) => {
  if (event.type === "session.status") {
    return event.properties?.status?.type === "busy" ? ["working"] : undefined;
  }
  switch (event.type) {
    case "session.created": return ["idle"];
    case "session.idle": return ["done"];
    case "session.deleted": return ["clear"];
    case "session.error": return ["error"];
    case "permission.asked": return ["blocked", "permission"];
    case "permission.replied": return ["working"];
    default: return undefined;
  }
};

export const Fastty = async () => ({
  event({ event }) {
    const status = stateForEvent(event);
    if (status) publish(...status);
  },
});
