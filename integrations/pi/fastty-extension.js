import { spawn } from "node:child_process";

const publish = (state, kind) => {
  const args = ["agent-status", state, "pi"];
  if (kind) args.push(kind);
  const child = spawn("fastty", args, { stdio: "ignore" });
  child.on("error", () => {});
};

export default function fasttyExtension(pi) {
  pi.on("session_start", () => publish("idle"));
  pi.on("session_shutdown", () => publish("clear"));
  pi.on("agent_start", () => publish("working"));
  pi.on("agent_settled", () => publish("done"));
}
