import { Component, type ReactNode } from "react";
import type { LibraryProblem } from "../api";
import { useStore } from "../store";
import { TitleBar } from "./TitleBar";

/** Shown instead of the library when it couldn't open, with a way to try again. */
export function LibraryProblemScreen({ problem }: { problem: LibraryProblem }) {
  const { retryLibrary, reveal } = useStore.getState();
  return (
    <main className="panel empty problem">
      <h2>Your library couldn’t be opened</h2>
      <p>
        {problem.path ? (
          <>
            Tonality keeps your photos in <span className="problem-path">{problem.path}</span>.
          </>
        ) : (
          "Tonality couldn’t find where to keep your photos."
        )}{" "}
        Check that the folder exists and can be changed, then try again.
      </p>
      <p className="problem-detail">{problem.message}</p>
      <div className="empty-actions">
        <button className="button primary" onClick={() => void retryLibrary()}>
          Try again
        </button>
        {problem.path && (
          <button className="button" onClick={() => void reveal(problem.path!)}>
            Show folder
          </button>
        )}
      </div>
    </main>
  );
}

/**
 * Catches an error while drawing the interface, so the window shows what
 * happened and a way back instead of going blank.
 */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <>
        <TitleBar />
        <main className="panel empty problem">
          <h2>Something went wrong</h2>
          <p>Tonality hit a problem drawing this screen. Your photos and edits are safe in the library.</p>
          <p className="problem-detail">{error.message || String(error)}</p>
          <div className="empty-actions">
            <button className="button primary" onClick={() => window.location.reload()}>
              Reload
            </button>
          </div>
        </main>
      </>
    );
  }
}
