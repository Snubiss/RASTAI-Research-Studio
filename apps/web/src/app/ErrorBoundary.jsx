// Catches a crash in one panel, so the rest of the window keeps working (layout frontend-and-plugins.md 2.7). It shows
// RRS-U1004 with a button that renders the panel again. A class, because React's error boundaries are classes.

import { Button, DiagnosticView } from "@rastai/ui";
import { Component } from "react";

import { CODE_PANEL_CRASHED } from "../constants.js";
import { pluginDiagnostic } from "@rastai/plugin";

/** @typedef {import("react").ReactNode} ReactNode */

/**
 * @typedef {object} ErrorBoundaryProps
 * @property {string} panelId
 * @property {ReactNode} children
 */

/**
 * @extends {Component<ErrorBoundaryProps, { error: Error | null }>}
 */
export class ErrorBoundary extends Component {
  /** @param {ErrorBoundaryProps} props */
  constructor(props) {
    super(props);
    /** @type {{ error: Error | null }} */
    this.state = { error: null };
    this.reloadPanel = this.reloadPanel.bind(this);
  }

  /** @param {Error} error */
  static getDerivedStateFromError(error) {
    return { error };
  }

  /** Renders the panel again; a panel that crashes again shows the same message. */
  reloadPanel() {
    this.setState({ error: null });
  }

  render() {
    const { error } = this.state;
    if (error === null) {
      return this.props.children;
    }
    const reason = error instanceof Error ? error.message : String(error);
    const diagnostic = pluginDiagnostic(CODE_PANEL_CRASHED, { panel_id: this.props.panelId, reason });
    return (
      <div className="rr-panel-error">
        <DiagnosticView diagnostic={diagnostic} />
        <Button variant="secondary" onClick={this.reloadPanel}>
          Reload panel
        </Button>
      </div>
    );
  }
}
