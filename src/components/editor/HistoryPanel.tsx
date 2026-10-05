import { useState } from "react";
import { ChevronDown, Download, GitBranch, GitBranchPlus } from "lucide-react";
import { Step } from "../../api";
import { ago } from "../../format";
import { useStore } from "../../store";
import { menuBelow } from "../Toolbar";
import { NameInput } from "./NameInput";

function StepRow(props: { step: Step; state: "done" | "current" | "undone"; onGo: () => void; onBranch: () => void }) {
  const { step, state } = props;
  return (
    <li className={`step ${state}`}>
      <button className="step-main" aria-current={state === "current" ? "step" : undefined} onClick={props.onGo}>
        <span className="step-dot" />
        <span className="step-label">{step.label}</span>
        <span className="step-time">{ago(step.createdAt)}</span>
      </button>
      <button className="icon-button small step-branch" title="Start a branch from this step" aria-label="Start a branch from this step" onClick={props.onBranch}>
        <GitBranchPlus size={14} />
      </button>
      {step.forks.length > 0 && (
        <p className="step-fork">
          <GitBranch size={12} />
          {step.forks.join(", ")} {step.forks.length === 1 ? "branches" : "branch"} off here
        </p>
      )}
      {step.exports.map((file) => (
        <button
          key={file.path}
          className="step-fork step-export"
          title={`${file.path}\nShow in file manager`}
          onClick={() => void useStore.getState().reveal(file.path)}
        >
          <Download size={12} />
          <span>Exported as {file.path.split(/[/\\]/).pop()}</span>
          <span className="step-time">{ago(file.createdAt)}</span>
        </button>
      ))}
    </li>
  );
}

/**
 * The photo's edit history: the steps of the current branch, newest first,
 * and the branches to switch between. A step that was exported says so, with
 * the file it became. Click a step to go back to it; branch
 * from any step to take the edit in another direction without losing this one.
 */
export function HistoryPanel() {
  const history = useStore((s) => s.editor.history);
  const [renaming, setRenaming] = useState(false);
  const s = useStore.getState();
  if (!history) return <div className="history waiting" />;

  const branch = history.branches.find((b) => b.id === history.branchId)!;
  const headIndex = history.steps.findIndex((step) => step.id === history.headId);
  const undone = history.steps.length - 1 - headIndex;

  const startBranch = async (stepId?: number) => {
    await s.newBranch(stepId);
    // Straight into naming it, while you still know what it is for.
    setRenaming(true);
  };

  const branchMenu = (event: React.MouseEvent) =>
    menuBelow(event, [
      ...history.branches.map((b) => ({
        label: b.name,
        checked: b.id === history.branchId,
        run: () => void s.switchBranch(b.id),
      })),
      "separator",
      { label: "Rename this branch", run: () => setRenaming(true) },
      {
        label: "Delete this branch",
        danger: true,
        disabled: history.branches.length < 2,
        run: () => void s.deleteBranch(branch.id),
      },
    ]);

  return (
    <div className="history">
      <div className="branch-bar">
        {renaming ? (
          <NameInput
            name={branch.name}
            label="Branch name"
            onDone={(name) => {
              setRenaming(false);
              void s.renameBranch(branch.id, name);
            }}
          />
        ) : (
          <button className="button branch-picker" title="Switch, rename or delete branches" onClick={branchMenu}>
            <GitBranch size={15} />
            <span>{branch.name}</span>
            <ChevronDown size={14} />
          </button>
        )}
        <button className="button" title="Start a new branch from the current step" onClick={() => void startBranch()}>
          <GitBranchPlus size={15} /> Branch
        </button>
      </div>

      {undone > 0 && (
        <p className="history-note">
          {undone === 1 ? "1 step is" : `${undone} steps are`} undone. Your next edit replaces {undone === 1 ? "it" : "them"}; start a
          branch first to keep both.
        </p>
      )}

      <ol className="steps">
        {history.steps
          .map((step, index) => (
            <StepRow
              key={step.id}
              step={step}
              state={index === headIndex ? "current" : index > headIndex ? "undone" : "done"}
              onGo={() => void s.goToStep(step.id)}
              onBranch={() => void startBranch(step.id)}
            />
          ))
          .reverse()}
      </ol>
    </div>
  );
}
