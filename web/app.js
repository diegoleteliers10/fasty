import init, { FasttyVt } from "./pkg/fastty_wasm.js";

// Process icons mapping matching Fastty's Deck Icons
const ICONS = {
  vim: "",
  nvim: "",
  node: "",
  npm: "",
  cargo: "",
  rustc: "",
  git: "",
  docker: "",
  python: "",
  python3: "",
  ssh: "󰣀",
  zsh: "",
  bash: "",
  fish: "",
  sh: "",
};

function getProcessIcon(title) {
  const clean = (title || "").trim().toLowerCase();
  for (const [k, icon] of Object.entries(ICONS)) {
    if (clean.includes(k)) return icon;
  }
  return "";
}

const DEFAULT_FONT_SIZE = 14;
const MIN_FONT_SIZE = 8;
const MAX_FONT_SIZE = 32;

class FasttyWebClient {
  constructor() {
    this.vt = null;
    this.ws = null;
    this.activeSessionId = null;
    this.sessions = [];
    this.canvas = document.getElementById("terminal-canvas");
    this.wrapper = document.getElementById("terminal-wrapper");
    this.tabsEl = document.getElementById("session-tabs");
    this.statusPill = document.getElementById("connection-status");
    this.reconnectBtn = document.getElementById("reconnect-btn");
    this.overlayMsg = document.getElementById("overlay-msg");
    this.overlayTitle = document.getElementById("overlay-title");
    this.overlayDesc = document.getElementById("overlay-desc");
    this.sessionInfoEl = document.getElementById("session-info");
    this.dimEl = document.getElementById("term-dimensions");
    this.cwdEl = document.getElementById("term-cwd");
    this.scrollStatusEl = document.getElementById("scroll-status");
    this.jumpBottomBtn = document.getElementById("jump-bottom-btn");
    this.scrollbarTrack = document.getElementById("scrollbar-track");
    this.scrollbarThumb = document.getElementById("scrollbar-thumb");

    // Font size controls
    this.fontSizeDisplay = document.getElementById("font-size-display");
    this.fontDecBtn = document.getElementById("font-decrease-btn");
    this.fontIncBtn = document.getElementById("font-increase-btn");
    this.fontResetBtn = document.getElementById("font-reset-btn");

    const savedFontSize = parseInt(localStorage.getItem("fastty_web_font_size"), 10);
    this.fontSize = (!isNaN(savedFontSize) && savedFontSize >= MIN_FONT_SIZE && savedFontSize <= MAX_FONT_SIZE)
      ? savedFontSize
      : DEFAULT_FONT_SIZE;

    this.fontFamily = "'JetBrains Mono', monospace";
    this.dpr = window.devicePixelRatio || 1;

    // Scroll state
    this.wheelAccumulator = 0;
    this.isDraggingScrollbar = false;
    this.dragStartY = 0;
    this.dragStartOffset = 0;
    this.touchStartY = 0;

    this.updateFontSizeUI();
    this.initEvents();
  }

  async start() {
    // 1. Initialize WebAssembly module
    await init();
    this.vt = new FasttyVt(80, 24, 10000);

    // 2. Connect to WebSocket Daemon Bridge
    this.connectWs();

    // 3. Start 60fps render loop
    this.renderLoop();
  }

  setFontSize(newSize) {
    this.fontSize = Math.max(MIN_FONT_SIZE, Math.min(MAX_FONT_SIZE, newSize));
    localStorage.setItem("fastty_web_font_size", this.fontSize);
    this.updateFontSizeUI();
    if (this.vt) {
      this.vt.render_canvas(this.canvas, this.fontFamily, this.fontSize, this.dpr);
    }
  }

  updateFontSizeUI() {
    if (this.fontSizeDisplay) {
      this.fontSizeDisplay.textContent = `${this.fontSize}px`;
    }
  }

  initEvents() {
    // Font controls
    this.fontDecBtn.addEventListener("click", () => this.setFontSize(this.fontSize - 1));
    this.fontIncBtn.addEventListener("click", () => this.setFontSize(this.fontSize + 1));
    this.fontResetBtn.addEventListener("click", () => this.setFontSize(DEFAULT_FONT_SIZE));

    this.reconnectBtn.addEventListener("click", () => {
      if (this.ws) {
        this.ws.close();
      }
      this.connectWs();
    });

    this.jumpBottomBtn.addEventListener("click", () => {
      if (this.vt) {
        this.vt.scroll_to_bottom();
        this.updateScrollbarUI();
      }
    });

    window.addEventListener("resize", () => {
      this.dpr = window.devicePixelRatio || 1;
      if (this.vt) {
        this.vt.render_canvas(this.canvas, this.fontFamily, this.fontSize, this.dpr);
      }
    });

    // Keyboard Input & Shortcuts
    this.wrapper.addEventListener("keydown", (e) => {
      // Zoom shortcuts: Ctrl/Cmd + Plus/Minus/0
      if (e.ctrlKey || e.metaKey) {
        if (e.key === "=" || e.key === "+") {
          e.preventDefault();
          this.setFontSize(this.fontSize + 1);
          return;
        } else if (e.key === "-") {
          e.preventDefault();
          this.setFontSize(this.fontSize - 1);
          return;
        } else if (e.key === "0") {
          e.preventDefault();
          this.setFontSize(DEFAULT_FONT_SIZE);
          return;
        }
      }

      // Scroll shortcuts: Shift + PageUp/PageDown, Shift + Home/End, Shift + Up/Down
      if (this.vt && e.shiftKey) {
        if (e.key === "PageUp") {
          e.preventDefault();
          this.vt.scroll_page_up();
          this.updateScrollbarUI();
          return;
        } else if (e.key === "PageDown") {
          e.preventDefault();
          this.vt.scroll_page_down();
          this.updateScrollbarUI();
          return;
        } else if (e.key === "Home") {
          e.preventDefault();
          this.vt.scroll_to_top();
          this.updateScrollbarUI();
          return;
        } else if (e.key === "End") {
          e.preventDefault();
          this.vt.scroll_to_bottom();
          this.updateScrollbarUI();
          return;
        } else if (e.key === "ArrowUp") {
          e.preventDefault();
          this.vt.scroll_display(1);
          this.updateScrollbarUI();
          return;
        } else if (e.key === "ArrowDown") {
          e.preventDefault();
          this.vt.scroll_display(-1);
          this.updateScrollbarUI();
          return;
        }
      }

      if (!this.vt || !this.activeSessionId || !this.ws || this.ws.readyState !== WebSocket.OPEN) {
        return;
      }

      // Handle copy/paste shortcuts
      if ((e.ctrlKey || e.metaKey) && e.key === "c" && window.getSelection().toString()) {
        return; // Allow standard browser copy
      }

      const seq = FasttyVt.encode_key(e.key, e.ctrlKey, e.altKey, e.shiftKey, e.metaKey);
      if (seq) {
        e.preventDefault();
        e.stopPropagation();
        this.sendInput(seq);
      }
    });

    // Mouse Wheel / Trackpad Scroll
    this.wrapper.addEventListener("wheel", (e) => {
      if (!this.vt) return;
      e.preventDefault();

      this.wheelAccumulator += e.deltaY;
      const lineHeight = Math.max(12, this.fontSize * 1.25);
      const lines = Math.trunc(this.wheelAccumulator / lineHeight);

      if (lines !== 0) {
        this.wheelAccumulator -= lines * lineHeight;
        // Positive deltaY (scroll down gesture) -> scroll down (-lines)
        // Negative deltaY (scroll up gesture) -> scroll up into history (+lines)
        this.vt.scroll_display(-lines);
        this.updateScrollbarUI();
      }
    }, { passive: false });

    // Touch Swipe Scrolling
    this.wrapper.addEventListener("touchstart", (e) => {
      if (e.touches.length === 1) {
        this.touchStartY = e.touches[0].clientY;
      }
    }, { passive: true });

    this.wrapper.addEventListener("touchmove", (e) => {
      if (!this.vt || e.touches.length !== 1) return;
      const currentY = e.touches[0].clientY;
      const deltaY = currentY - this.touchStartY;
      const lineHeight = Math.max(12, this.fontSize * 1.25);
      const lines = Math.trunc(deltaY / lineHeight);
      if (lines !== 0) {
        this.touchStartY = currentY;
        this.vt.scroll_display(lines);
        this.updateScrollbarUI();
      }
    }, { passive: true });

    // Scrollbar Drag & Click
    this.scrollbarThumb.addEventListener("mousedown", (e) => {
      e.preventDefault();
      e.stopPropagation();
      this.isDraggingScrollbar = true;
      this.dragStartY = e.clientY;
      this.dragStartOffset = this.vt ? this.vt.scroll_offset() : 0;
      document.body.style.userSelect = "none";
    });

    this.scrollbarTrack.addEventListener("mousedown", (e) => {
      if (e.target === this.scrollbarThumb || !this.vt) return;
      const rect = this.scrollbarTrack.getBoundingClientRect();
      const clickY = e.clientY - rect.top;
      const trackHeight = rect.height;
      const ratio = 1 - Math.max(0, Math.min(1, clickY / trackHeight));
      const maxScroll = this.vt.max_scroll_offset();
      this.vt.scroll_to(Math.round(ratio * maxScroll));
      this.updateScrollbarUI();
    });

    window.addEventListener("mousemove", (e) => {
      if (!this.isDraggingScrollbar || !this.vt) return;
      const maxScroll = this.vt.max_scroll_offset();
      if (maxScroll === 0) return;

      const trackHeight = this.scrollbarTrack.clientHeight;
      const deltaY = e.clientY - this.dragStartY;
      const deltaRatio = deltaY / trackHeight;
      const deltaLines = Math.round(deltaRatio * maxScroll);

      const targetOffset = Math.max(0, Math.min(maxScroll, this.dragStartOffset - deltaLines));
      this.vt.scroll_to(targetOffset);
      this.updateScrollbarUI();
    });

    window.addEventListener("mouseup", () => {
      if (this.isDraggingScrollbar) {
        this.isDraggingScrollbar = false;
        document.body.style.userSelect = "";
      }
    });

    // Focus wrapper on click
    this.wrapper.addEventListener("click", () => {
      this.wrapper.focus();
    });
  }

  updateScrollbarUI() {
    if (!this.vt) return;
    const offset = this.vt.scroll_offset();
    const maxScroll = this.vt.max_scroll_offset();

    if (maxScroll === 0) {
      this.scrollbarThumb.style.display = "none";
      this.scrollStatusEl.classList.add("hidden");
      this.jumpBottomBtn.classList.add("hidden");
      return;
    }

    this.scrollbarThumb.style.display = "block";
    const trackHeight = this.scrollbarTrack.clientHeight || 500;
    const thumbHeight = Math.max(24, (trackHeight / (maxScroll + trackHeight)) * trackHeight);
    this.scrollbarThumb.style.height = `${thumbHeight}px`;

    const availableTrack = trackHeight - thumbHeight;
    const ratio = 1 - (offset / maxScroll);
    const topPos = ratio * availableTrack;
    this.scrollbarThumb.style.top = `${topPos}px`;

    if (offset > 0) {
      this.scrollStatusEl.textContent = `Scroll: ${offset} / ${maxScroll}`;
      this.scrollStatusEl.classList.remove("hidden");
      this.jumpBottomBtn.classList.remove("hidden");
    } else {
      this.scrollStatusEl.classList.add("hidden");
      this.jumpBottomBtn.classList.add("hidden");
    }
  }

  connectWs() {
    this.setStatus("connecting", "Connecting...");
    this.showOverlay("Connecting to fastty daemon...", "Ensure fastty is running with its socket active.");

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const wsUrl = `${protocol}//${window.location.host}/ws`;

    this.ws = new WebSocket(wsUrl);

    this.ws.onopen = () => {
      this.setStatus("connected", "Connected");
      this.hideOverlay();
      // Handshake and subscribe to sessions
      this.ws.send(JSON.stringify({ cmd: "hello" }) + "\n");
      this.ws.send(JSON.stringify({ cmd: "subscribe_sessions" }) + "\n");
    };

    this.ws.onmessage = (event) => {
      try {
        const lines = event.data.split("\n");
        for (const line of lines) {
          if (!line.trim()) continue;
          const msg = JSON.parse(line);
          this.handleDaemonMessage(msg);
        }
      } catch (err) {
        console.error("Error parsing daemon message:", err, event.data);
      }
    };

    this.ws.onclose = () => {
      this.setStatus("disconnected", "Disconnected");
      this.showOverlay("Disconnected from daemon", "Click ↻ or restart fastty.");
    };

    this.ws.onerror = (err) => {
      console.error("WebSocket error:", err);
    };
  }

  handleDaemonMessage(msg) {
    switch (msg.event) {
      case "hello":
        console.log(`⚡ Connected to fastty daemon v${msg.fastty_version} (protocol v${msg.version})`);
        break;

      case "sessions":
        this.sessions = msg.sessions || [];
        this.renderTabs();
        if (this.sessions.length > 0 && !this.activeSessionId) {
          this.attachToSession(this.sessions[0].id);
        }
        break;

      case "session_added":
        this.sessions = this.sessions.filter(s => s.id !== msg.session.id);
        this.sessions.push(msg.session);
        this.renderTabs();
        if (!this.activeSessionId) {
          this.attachToSession(msg.session.id);
        }
        break;

      case "session_removed":
        this.sessions = this.sessions.filter(s => s.id !== msg.id);
        this.renderTabs();
        if (this.activeSessionId === msg.id) {
          if (this.sessions.length > 0) {
            this.attachToSession(this.sessions[0].id);
          } else {
            this.activeSessionId = null;
            this.showOverlay("No active sessions", "Open a tab in fastty to connect.");
          }
        }
        break;

      case "session_updated":
        const idx = this.sessions.findIndex(s => s.id === msg.session.id);
        if (idx !== -1) {
          this.sessions[idx] = msg.session;
          this.renderTabs();
          if (this.activeSessionId === msg.session.id) {
            this.updateStatusBar(msg.session);
          }
        }
        break;

      case "attached":
        if (msg.id === this.activeSessionId) {
          this.vt.resize(msg.cols, msg.rows);
          this.dimEl.textContent = `${msg.cols} × ${msg.rows}`;
        }
        break;

      case "snapshot":
      case "output":
        if (msg.id === this.activeSessionId && msg.data) {
          const binary = atob(msg.data);
          const bytes = new Uint8Array(binary.length);
          for (let i = 0; i < binary.length; i++) {
            bytes[i] = binary.charCodeAt(i);
          }
          this.vt.feed_bytes(bytes);
          this.updateScrollbarUI();
        }
        break;

      case "closed":
        if (msg.id === this.activeSessionId) {
          this.showOverlay(`Session ${msg.id} Closed`, "Select another tab above.");
        }
        break;

      case "error":
        console.warn("Daemon returned error:", msg.code, msg.message);
        break;
    }
  }

  attachToSession(id) {
    if (this.activeSessionId === id) return;
    if (this.activeSessionId && this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ cmd: "detach", id: this.activeSessionId }) + "\n");
    }

    this.activeSessionId = id;
    this.renderTabs();

    const session = this.sessions.find(s => s.id === id);
    if (session) {
      this.updateStatusBar(session);
      this.vt.resize(session.cols, session.rows);
    }

    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ cmd: "attach", id, mode: "read_write" }) + "\n");
    }

    this.wrapper.focus();
  }

  sendInput(str) {
    if (!this.activeSessionId || !this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    const b64 = btoa(unescape(encodeURIComponent(str)));
    this.ws.send(JSON.stringify({ cmd: "write", id: this.activeSessionId, data: b64 }) + "\n");
    this.vt.scroll_to_bottom();
    this.updateScrollbarUI();
  }

  renderTabs() {
    if (this.sessions.length === 0) {
      this.tabsEl.innerHTML = '<div class="tab loading-tab">No sessions active</div>';
      return;
    }

    this.tabsEl.innerHTML = "";
    for (const session of this.sessions) {
      const tab = document.createElement("div");
      tab.className = `tab ${session.id === this.activeSessionId ? "active" : ""}`;
      const icon = getProcessIcon(session.title);
      tab.innerHTML = `<span class="tab-icon">${icon}</span> [${session.id}] ${session.title}`;
      tab.addEventListener("click", () => this.attachToSession(session.id));
      this.tabsEl.appendChild(tab);
    }
  }

  updateStatusBar(session) {
    this.sessionInfoEl.textContent = `Session [${session.id}] • ${session.title}`;
    this.dimEl.textContent = `${session.cols} × ${session.rows}`;
    this.cwdEl.textContent = session.cwd || "~";
  }

  renderLoop() {
    if (this.vt && this.vt.is_dirty()) {
      this.vt.render_canvas(this.canvas, this.fontFamily, this.fontSize, this.dpr);
    }
    requestAnimationFrame(() => this.renderLoop());
  }

  setStatus(type, text) {
    this.statusPill.className = `status-pill status-${type}`;
    this.statusPill.querySelector(".text").textContent = text;
  }

  showOverlay(title, desc) {
    this.overlayTitle.textContent = title;
    this.overlayDesc.textContent = desc;
    this.overlayMsg.classList.remove("hidden");
  }

  hideOverlay() {
    this.overlayMsg.classList.add("hidden");
  }
}

// Bootstrap
window.addEventListener("DOMContentLoaded", () => {
  const client = new FasttyWebClient();
  client.start().catch(console.error);
});
