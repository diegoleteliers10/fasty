import os
import subprocess
import threading


class _Status:
    def __init__(self):
        self._lock = threading.Lock()
        self._session_id = None
        self._retired = set()
        self._pending = {}

    def _accept(self, session_id, platform=None, parent_session_id=None):
        return (
            isinstance(session_id, str)
            and bool(session_id)
            and session_id == self._session_id
            and platform in (None, "", "cli")
            and not parent_session_id
        )

    def _start(self, session_id, platform, parent_session_id):
        if (
            self._session_id is None
            and isinstance(session_id, str)
            and session_id
            and session_id not in self._retired
            and platform == "cli"
            and not parent_session_id
        ):
            self._session_id = session_id
        return self._accept(session_id, platform, parent_session_id)

    def _publish(self, state, kind=None):
        args = ["fastty", "agent-status", state, "hermes"]
        if kind is not None:
            args.append(kind)
        try:
            subprocess.run(
                args,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=0.5,
                check=False,
            )
        except (OSError, subprocess.SubprocessError):
            pass

    def _working(self):
        if self._pending:
            request_id = next(reversed(self._pending))
            self._publish("blocked", self._pending[request_id])
        else:
            self._publish("working")

    def start(self, session_id=None, platform=None, parent_session_id=None, **kwargs):
        with self._lock:
            if self._start(session_id, platform, parent_session_id):
                self._publish("idle")

    def working(self, session_id=None, platform=None, parent_session_id=None, **kwargs):
        with self._lock:
            if self._start(session_id, platform, parent_session_id):
                self._working()

    def end(
        self, session_id=None, platform=None, parent_session_id=None,
        completed=False, failed=False, interrupted=False, **kwargs
    ):
        with self._lock:
            if not self._accept(session_id, platform, parent_session_id):
                return
            self._pending.clear()
            if failed:
                self._publish("error")
            elif interrupted:
                self._publish("idle")
            elif completed:
                self._publish("done")
            else:
                self._publish("idle")

    def clear(self, session_id=None, platform=None, parent_session_id=None, **kwargs):
        with self._lock:
            if not self._accept(session_id, platform, parent_session_id):
                return
            self._retired.add(session_id)
            self._session_id = None
            self._pending.clear()
            self._publish("clear")

    def reset(
        self, session_id=None, platform=None, parent_session_id=None,
        old_session_id=None, **kwargs
    ):
        with self._lock:
            if (
                not isinstance(session_id, str)
                or not session_id
                or platform != "cli"
                or parent_session_id
                or (old_session_id and self._session_id not in (None, old_session_id))
            ):
                return
            if self._session_id and self._session_id != session_id:
                self._retired.add(self._session_id)
            self._session_id = session_id
            self._pending.clear()
            self._publish("clear")

    def request(
        self, kind=None, request_id=None, session_id=None, platform=None,
        parent_session_id=None, **kwargs
    ):
        kinds = {"approval": "permission", "clarify": "question", "sudo": "auth"}
        with self._lock:
            if (
                not self._accept(session_id, platform, parent_session_id)
                or not isinstance(request_id, str)
                or not request_id
                or kind not in kinds
            ):
                return
            self._pending[request_id] = kinds[kind]
            self._working()

    def resolved(
        self, request_id=None, session_id=None, platform=None,
        parent_session_id=None, **kwargs
    ):
        with self._lock:
            if (
                not self._accept(session_id, platform, parent_session_id)
                or not isinstance(request_id, str)
                or request_id not in self._pending
            ):
                return
            del self._pending[request_id]
            self._working()


def register(ctx):
    if not os.environ.get("FASTTY_SESSION_TOKEN"):
        return
    if os.environ.get("HERMES_PLATFORM", "cli") != "cli":
        return
    status = _Status()
    for name, callback in (
        ("on_session_start", status.start),
        ("pre_llm_call", status.working),
        ("on_session_end", status.end),
        ("on_session_finalize", status.clear),
        ("on_session_reset", status.reset),
        ("on_human_input_request", status.request),
        ("on_human_input_resolved", status.resolved),
    ):
        ctx.register_hook(name, callback)
