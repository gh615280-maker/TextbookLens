import { Component, type ReactNode } from 'react';

import { formatMessage } from '../lib/i18n';

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
}

export class ErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  public state: ErrorBoundaryState = { hasError: false };

  public static getDerivedStateFromError(): ErrorBoundaryState {
    return { hasError: true };
  }

  private retry = (): void => {
    this.setState({ hasError: false });
  };

  public render(): ReactNode {
    if (this.state.hasError) {
      return (
        <section className="error-boundary" role="alert">
          <p>{formatMessage('zh-CN', 'error.message')}</p>
          <button type="button" onClick={this.retry}>
            {formatMessage('zh-CN', 'error.retry')}
          </button>
        </section>
      );
    }

    return this.props.children;
  }
}
