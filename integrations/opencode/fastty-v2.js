import { Plugin } from "@opencode/plugin/tui";
import { spawn } from "node:child_process";

const publish = (state, kind) => new Promise((resolve) => {
  const args = ["agent-status", state, "opencode"];
  if (kind) args.push(kind);
  const child = spawn("fastty", args, { stdio: "ignore" });
  const timeout = setTimeout(() => child.kill("SIGKILL"), 2000);
  child.once("error", () => {
    clearTimeout(timeout);
    resolve();
  });
  child.once("close", () => {
    clearTimeout(timeout);
    resolve();
  });
}).then(() => undefined, () => undefined);

const statusForSession = (session) => {
  const kinds = [...session.requests.values()];
  if (kinds.includes("permission")) return ["blocked", "permission"];
  if (kinds.length) return ["blocked", "question"];
  return session.status;
};

const priority = { blocked: 5, working: 4, error: 3, done: 2, idle: 1 };

const aggregateStatus = (sessions) => {
  let status = ["clear"];
  for (const session of sessions.values()) {
    const next = statusForSession(session);
    if ((priority[next[0]] ?? 0) > (priority[status[0]] ?? 0)
      || (next[0] === "blocked" && next[1] === "permission")) status = next;
  }
  return status;
};

const stateForEvent = (event) => {
  switch (event.type) {
    case "session.created": return ["idle"];
    case "session.execution.started":
    case "session.retry.scheduled": return ["working"];
    case "session.execution.succeeded": return ["done"];
    case "session.execution.failed": return ["error"];
    case "session.execution.interrupted": return ["idle"];
    case "session.deleted": return ["clear"];
    case "permission.asked": return ["blocked", "permission"];
    case "form.created": return ["blocked", "question"];
    case "permission.replied":
    case "form.replied":
    case "form.cancelled": return ["working"];
    default: return undefined;
  }
};

export default Plugin.define({
  id: "fastty.agent-status",
  setup(context) {
    if (!process.env.FASTTY_SESSION_TOKEN) return;
    const sessions = new Map();
    let lastStatus;
    let pending = Promise.resolve();
    let disposed = false;

    const visibleRoots = () => {
      const current = context.ui.router.current();
      const ids = context.ui.tabs.list().map((tab) => tab.sessionID);
      if (current.type === "session") ids.push(current.sessionID);
      return new Set(ids.map((id) => context.data.session.root(id)));
    };

    const reconcile = (refresh) => {
      const roots = visibleRoots();
      for (const id of sessions.keys()) {
        if (!roots.has(id)) sessions.delete(id);
      }
      for (const id of roots) {
        if (!context.data.session.get(id)) continue;
        const existing = sessions.get(id);
        if (existing && !refresh) continue;
        const running = context.data.session.status(id) === "running";
        const session = existing ?? { status: [running ? "working" : "idle"], requests: new Map() };
        if (running) session.status = ["working"];
        if (!existing || session.status[0] === "working") {
          const permissions = context.data.session.permission.list(id);
          const forms = context.data.session.form.list(id, context.location);
          for (const [prefix, requests, kind] of [
            ["permission", permissions, "permission"],
            ["form", forms, "question"],
          ]) {
            if (requests === undefined) continue;
            for (const key of session.requests.keys()) {
              if (key.startsWith(`${prefix}:`)) session.requests.delete(key);
            }
            for (const request of requests) session.requests.set(`${prefix}:${request.id}`, kind);
          }
          if (session.requests.size) session.status = ["working"];
          else if (!running && session.status[0] === "working") session.status = ["done"];
        }
        sessions.set(id, session);
      }
    };

    const emit = async () => {
      const status = aggregateStatus(sessions);
      const key = status.join(":");
      if (key === lastStatus) return;
      lastStatus = key;
      await publish(...status);
    };

    const handle = async (event) => {
      if (disposed) return;
      reconcile(!event);
      if (!event) return emit();
      const next = stateForEvent(event);
      const data = event.data;
      const id = data?.sessionID ?? data?.form?.sessionID;
      const session = sessions.get(id);
      if (!next || !session) return emit();
      if (next[0] === "clear") {
        sessions.delete(id);
      } else if (event.type === "permission.asked") {
        session.requests.set(`permission:${data.id}`, "permission");
        session.status = ["working"];
      } else if (event.type === "form.created") {
        session.requests.set(`form:${data.form.id}`, "question");
        session.status = ["working"];
      } else if (event.type === "permission.replied" || event.type === "form.replied" || event.type === "form.cancelled") {
        const key = event.type === "permission.replied" ? `permission:${data.requestID}` : `form:${data.id}`;
        if (session.requests.delete(key)) session.status = next;
      } else {
        if (["done", "error", "idle"].includes(next[0])) session.requests.clear();
        session.status = next;
      }
      await emit();
    };

    const enqueue = (event) => {
      pending = pending.then(() => handle(event)).then(() => undefined, () => undefined);
    };
    const stop = context.data.listen(({ details }) => enqueue(details));
    const interval = setInterval(() => enqueue(), 1000);
    enqueue();
    return () => {
      disposed = true;
      stop();
      clearInterval(interval);
      pending = pending.then(() => publish("clear")).then(() => undefined, () => undefined);
    };
  },
});
