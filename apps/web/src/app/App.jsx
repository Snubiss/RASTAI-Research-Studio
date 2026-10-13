// The frame (layout frontend-and-plugins.md 8.1; design 3.1 to 3.5): the top bar, the section rail, the workspace and
// the status bar, plus the palette, the dialogs and the toasts. Signed out, the frame is replaced by SignedOutScreen
// from @rastai/ui/security. The global keys (design 3.7) are one window listener, here.

import { SignedOutScreen } from "@rastai/ui/security";
import { Toast, dismissToast } from "@rastai/ui";
import { useStore } from "@rastai/plugin";
import { useEffect, useRef } from "react";

import { FOCUS_REGIONS, GO_PREFIX_TIMEOUT_MS } from "../constants.js";
import { CommandPalette } from "../frame/CommandPalette.jsx";
import { Overlays } from "../frame/Overlays.jsx";
import { SectionRail } from "../frame/SectionRail.jsx";
import { StatusBar } from "../frame/StatusBar.jsx";
import { TopBar } from "../frame/TopBar.jsx";
import { NotAvailableSection } from "../sections/NotAvailableSection.jsx";
import { navigateTo } from "../routing/navigate.js";
import { sectionRoute } from "../routing/route.js";
import { sectionById } from "../plugin-host/plugin-registry.js";
import {
  ACTION_FOCUS_REGION,
  ACTION_GO_PREFIX,
  ACTION_PALETTE,
  ACTION_SHORTCUTS,
  isTextField,
  keyStep,
} from "./keyboard.js";
import { ConnectionBanner } from "./ConnectionBanner.jsx";
import { ShellProvider, useShell } from "./shell-context.js";

/** @typedef {import("./shell.js").Shell} Shell */

/**
 * @param {object} props
 * @param {Shell} props.shell
 */
export function App({ shell }) {
  return (
    <ShellProvider value={shell}>
      <Frame />
    </ShellProvider>
  );
}

function Frame() {
  const shell = useShell();
  const connection = useStore(shell.stores.connection);
  const goPending = useRef(false);
  const goTimer = useRef(/** @type {number | null} */ (null));

  useEffect(() => {
    /** @param {KeyboardEvent} event */
    function onKeyDown(event) {
      const step = keyStep(
        {
          key: event.key,
          ctrlOrMeta: event.ctrlKey || event.metaKey,
          alt: event.altKey,
          inTextField: isTextField(event.target),
        },
        goPending.current,
      );
      armGoPrefix(goPending, goTimer, step.goPending);
      if (step.action === "") {
        return;
      }
      event.preventDefault();
      runAction(shell, step.action);
    }
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      armGoPrefix(goPending, goTimer, false);
    };
  }, [shell]);

  if (connection.status === "signed_out") {
    return <SignedOutScreen />;
  }
  return (
    <div className="rr-frame">
      <a className="rr-skip-link" href="#main">
        Skip to content
      </a>
      <TopBar onOpenPalette={() => openPalette(shell)} />
      <ConnectionBanner />
      <div className="rr-frame__body">
        <SectionRail />
        <main id="main" className="rr-main" tabIndex={-1}>
          <RouteBody />
        </main>
      </div>
      <StatusBar />
      <Overlays />
      <CommandPalette />
      <Notices />
    </div>
  );
}

/**
 * Sets whether "G" is waiting for its letter. The wait ends on its own after GO_PREFIX_TIMEOUT_MS, so a stray G doesn't
 * turn the next letter into a jump.
 * @param {{ current: boolean }} pending
 * @param {{ current: number | null }} timer
 * @param {boolean} armed
 */
function armGoPrefix(pending, timer, armed) {
  if (timer.current !== null) {
    clearTimeout(timer.current);
    timer.current = null;
  }
  pending.current = armed;
  if (armed) {
    timer.current = setTimeout(() => {
      pending.current = false;
      timer.current = null;
    }, GO_PREFIX_TIMEOUT_MS);
  }
}

/** The current section's page, from the registered sections; a section no UI offers says so. */
function RouteBody() {
  const shell = useShell();
  const route = useStore(shell.stores.route);
  const section = sectionById(route.sectionId, shell.loadedPluginIds);
  if (section === null) {
    return <NotAvailableSection sectionId={route.sectionId} />;
  }
  return section.render({ path: route.path, query: route.query });
}

/** The toast list (failures from the daemon and from plugins). */
function Notices() {
  const shell = useShell();
  const notices = useStore(shell.stores.notices);
  return <Toast toasts={notices} onDismiss={(toastId) => shell.stores.notices.set(dismissToast(notices, toastId))} />;
}

/**
 * @param {Shell} shell
 */
function openPalette(shell) {
  shell.stores.palette.set({ open: true, query: "" });
}

/**
 * Runs one key action (app/keyboard.js).
 * @param {Shell} shell
 * @param {string} action
 */
function runAction(shell, action) {
  if (action === ACTION_PALETTE) {
    openPalette(shell);
  } else if (action === ACTION_SHORTCUTS) {
    shell.stores.overlay.set("shortcuts");
  } else if (action === ACTION_FOCUS_REGION) {
    focusNextRegion();
  } else if (action.startsWith(ACTION_GO_PREFIX)) {
    navigateTo(sectionRoute(action.slice(ACTION_GO_PREFIX.length)));
  }
}

/**
 * F6: the focus moves to the next region (the rail, the panels, the status bar) and lands on its first control.
 */
function focusNextRegion() {
  const regions = FOCUS_REGIONS.map((id) => document.getElementById(id)).filter((element) => element !== null);
  if (regions.length === 0) {
    return;
  }
  const current = regions.findIndex((region) => region.contains(document.activeElement));
  const next = regions[(current + 1) % regions.length];
  const firstControl = next.querySelector("a, button, input, select, textarea");
  (firstControl instanceof HTMLElement ? firstControl : next).focus();
}
