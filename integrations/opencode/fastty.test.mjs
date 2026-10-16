import test from "node:test";
import assert from "node:assert/strict";
import { Fastty } from "./fastty.js";

const { stateForEvent } = Fastty;

test("publishes working while an OpenCode session is busy", () => {
  assert.deepEqual(
    stateForEvent({ type: "session.status", properties: { status: { type: "busy" } } }),
    ["working"],
  );
});

test("ignores non-busy session status updates", () => {
  assert.equal(
    stateForEvent({ type: "session.status", properties: { status: { type: "idle" } } }),
    undefined,
  );
});
