"use client";
import React, { Component } from "react";
import { Button, Copy, Panel } from "./index";

interface Props {
  children: React.ReactNode;
  label?: string;
  onRetry?: () => void;
}

interface State {
  hasError: boolean;
  error: Error | null;
  resetKey: number;
}

// Class component: React Compiler does not compile class components, so this
// boundary itself stays compiler-clean by construction. Hooks around it remain
// functional and compiler-checked.
export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null, resetKey: 0 };

  static getDerivedStateFromError(_error: Error): Partial<State> {
    return { hasError: true, error: _error };
  }

  componentDidCatch(_error: Error, _errorInfo: React.ErrorInfo) {
    // Intentionally no side effects; the error is surfaced in the fallback UI.
  }

  private readonly handleRetry = () => {
    this.setState((prev) => ({
      hasError: false,
      error: null,
      resetKey: prev.resetKey + 1,
    }));
    this.props.onRetry?.();
  };

  render() {
    if (this.state.hasError) {
      return (
        <Panel title={this.props.label ?? "Error"}>
          <Copy>{this.state.error?.message ?? "An error occurred."}</Copy>
          {this.props.onRetry ? (
            <Button onPress={this.handleRetry}>Retry</Button>
          ) : null}
        </Panel>
      );
    }
    // `resetKey` changes on retry, remounting the wrapped children cleanly.
    return <React.Fragment key={this.state.resetKey}>{this.props.children}</React.Fragment>;
  }
}
