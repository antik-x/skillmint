import React from "react";
import { AlertCircle, RefreshCw } from "lucide-react";

interface Props {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}

interface State {
  hasError: boolean;
  error?: Error;
}

/**
 * SPEC-F2 T3: route-level error boundary to prevent white-screen crashes.
 */
export class ErrorBoundary extends React.Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[SkillMint] route error boundary caught:", error, info);
  }

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback;
      }
      return (
        <div className="flex h-full flex-col items-center justify-center bg-primary p-8 text-center">
          <AlertCircle className="h-12 w-12 text-danger" />
          <h2 className="mt-4 text-xl font-semibold text-primary">页面出错了</h2>
          <p className="mt-2 max-w-md text-sm text-secondary">
            该页面遇到意外错误。你可以点击下方按钮重新加载，或返回其它页面。
          </p>
          {this.state.error && (
            <pre className="mt-4 max-h-32 max-w-md overflow-auto rounded-md border border-danger/20 bg-secondary p-3 text-left text-xs text-tertiary">
              {this.state.error.message}
            </pre>
          )}
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="mt-6 inline-flex items-center gap-2 rounded-md bg-accent px-4 py-2 text-sm font-medium text-primary hover:bg-accent/90"
          >
            <RefreshCw className="h-4 w-4" />
            重新加载
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
