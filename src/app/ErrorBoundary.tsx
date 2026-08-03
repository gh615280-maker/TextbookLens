import { Component, type ContextType, type ReactNode } from 'react';

import { LanguageContext } from './LanguageProvider';

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
  public static contextType = LanguageContext;
  declare context: ContextType<typeof LanguageContext>;

  public state: ErrorBoundaryState = { hasError: false };

  public static getDerivedStateFromError(): ErrorBoundaryState {
    return { hasError: true };
  }

  private retry = (): void => {
    this.setState({ hasError: false });
  };

  public render(): ReactNode {
    if (this.state.hasError) {
      const message = this.context?.message;
      return (
        <section className="error-boundary" role="alert">
          <p>{message?.('error.message')}</p>
          <button type="button" onClick={this.retry}>
            {message?.('error.retry')}
          </button>
        </section>
      );
    }

    return this.props.children;
  }
}
