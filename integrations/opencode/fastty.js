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

const stateForEvent = (event) => {
  if (event.type === "session.status") {
    return ["busy", "retry"].includes(event.properties?.status?.type)
      ? ["working"]
      : undefined;
  }
  switch (event.type) {
    case "session.created": return ["idle"];
    case "session.idle": return ["done"];
    case "session.deleted": return ["clear"];
    case "session.error": return ["error"];
    case "permission.asked": return ["blocked", "permission"];
    case "question.asked": return ["blocked", "question"];
    case "permission.replied":
    case "question.replied":
    case "question.rejected": return ["working"];
    default: return undefined;
  }
};

const sessionIDForEvent = (event) => event.properties?.sessionID
  ?? event.properties?.info?.id;

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

const pruneCompleted = (sessions) => {
  const completed = [...sessions.entries()].filter(([, session]) =>
    session.status[0] !== "working" && session.requests.size === 0);
  for (const [id] of completed.slice(0, Math.max(0, completed.length - 64))) {
    sessions.delete(id);
  }
};

export const Fastty = async ({ client } = {}) => {
  if (!process.env.FASTTY_SESSION_TOKEN) return {};
  const sessions = new Map();
  const roots = new Map();
  let lastStatus;
  let pending = Promise.resolve();

  const isRoot = async (event, id) => {
    const info = event.properties?.info;
    if (info?.id === id) {
      roots.set(id, !info.parentID);
    } else if (!roots.has(id)) {
      if (!client?.session?.get) return false;
      const result = await client.session.get({ path: { id } }).then(
        (response) => response.data,
        () => undefined,
      );
      if (!result || result.id !== id) return false;
      roots.set(id, !result.parentID);
    }
    const root = roots.get(id);
    if (roots.size > 256) roots.delete(roots.keys().next().value);
    return root;
  };

  const handle = async (event) => {
    const next = stateForEvent(event);
    const id = sessionIDForEvent(event);
    if (!next || !id || !await isRoot(event, id)) return;
    if (next[0] === "clear") {
      sessions.delete(id);
      roots.delete(id);
    } else {
      const session = sessions.get(id) ?? { status: ["idle"], requests: new Map() };
      const properties = event.properties;
      if (next[0] === "blocked") {
        if (!properties.id) return;
        session.requests.set(`${event.type.split(".")[0]}:${properties.id}`, next[1]);
      } else if (["permission.replied", "question.replied", "question.rejected"].includes(event.type)) {
        if (!properties.requestID) return;
        const request = `${event.type.split(".")[0]}:${properties.requestID}`;
        if (!session.requests.delete(request)) return;
        session.status = next;
      } else {
        if (next[0] === "done" || next[0] === "error") session.requests.clear();
        if (!(next[0] === "done" && session.status[0] === "error")) session.status = next;
      }
      sessions.delete(id);
      sessions.set(id, session);
      pruneCompleted(sessions);
    }
    const status = aggregateStatus(sessions);
    const key = status.join(":");
    if (key === lastStatus) return;
    lastStatus = key;
    await publish(...status);
  };

  return {
    event({ event }) {
      pending = pending.then(() => handle(event)).then(() => undefined, () => undefined);
      return pending;
    },
  };
};

Fastty.stateForEvent = stateForEvent;
