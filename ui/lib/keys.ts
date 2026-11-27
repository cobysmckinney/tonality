// Window-level shortcuts must not steal keys from the control that has focus: Enter on a
// focused button presses that button, arrows move a slider, letters typed into a field are
// text. Keys a control doesn't use (a letter while a button has focus) stay shortcuts.

interface Element_ {
  closest(selector: string): Element_ | null;
  matches(selector: string): boolean;
  tagName: string;
  type?: string;
}

/** Take any key: text entry, and selects (arrows, type-ahead). */
const TYPING = [
  "textarea",
  "select",
  "[contenteditable]:not([contenteditable='false'])",
  "[role=textbox]",
  "[role=combobox]",
  "[role=listbox]",
].join(", ");
/** Take arrows (and Home/End/Page keys); all but the slider also take Enter and Space. */
const PICKERS = [
  "[role=radio]",
  "[role=tab]",
  "[role=menuitem]",
  "[role=menuitemcheckbox]",
  "[role=menuitemradio]",
  "[role=option]",
];
/** A separator that can be focused is a splitter: its arrows move it. */
const ARROWS = ["[role=slider]", "[role=separator][tabindex]", ...PICKERS].join(", ");
/** Take Enter and Space (activate). */
const ACTIVATE = [
  "button",
  "a[href]",
  "summary",
  "[role=button]",
  "[role=link]",
  "[role=checkbox]",
  "[role=switch]",
  ...PICKERS,
].join(", ");
/** Marks an element (a grid cell) whose keys are the window's shortcuts. */
const SHORTCUTS = "[data-shortcuts]";

const NAV = /^(Arrow|Home$|End$|Page)/;

function element(t: EventTarget | null): Element_ | null {
  const e = t as Partial<Element_> | null;
  return e && typeof e.closest === "function" && typeof e.matches === "function"
    ? (e as Element_)
    : null;
}

function inputOwns(type: string, key: string): boolean {
  switch (type) {
    case "checkbox":
      return key === " ";
    case "radio":
      return key === " " || NAV.test(key);
    case "range":
      return NAV.test(key);
    case "button":
    case "submit":
    case "reset":
    case "image":
    case "color":
    case "file":
      return key === " " || key === "Enter";
    default:
      return true; // text, number, date, …
  }
}

/** Does the focused control the key event came from use `key` itself? */
export function targetOwnsKey(target: EventTarget | null, key: string): boolean {
  const el = element(target);
  if (!el) return false;
  if (el.tagName === "INPUT") return inputOwns((el.type ?? "text").toLowerCase(), key);
  const hit = (sel: string) => {
    const c = el.closest(sel);
    return c !== null && !c.matches(SHORTCUTS);
  };
  if (hit(TYPING)) return true;
  if (NAV.test(key) && hit(ARROWS)) return true;
  return (key === "Enter" || key === " ") && hit(ACTIVATE);
}

/**
 * [`targetOwnsKey`], for the editor's own Enter and Space (apply the crop, pan while masking).
 *
 * A button keeps those two only while the keyboard put the focus on it, which is when it shows
 * a focus ring: Tab to Cancel, press Enter, and Cancel is pressed. A button that was clicked
 * with the mouse also still has focus on Linux and Windows, and there Enter has to go on
 * meaning "apply the crop" after clicking an aspect ratio, and Space "pan" after clicking a
 * brush mode. Fields, and every other key, are as [`targetOwnsKey`] says.
 */
export function focusedControlOwnsKey(target: EventTarget | null, key: string): boolean {
  if (!targetOwnsKey(target, key)) return false;
  if (key !== "Enter" && key !== " ") return true;
  const el = element(target);
  if (!el || el.tagName === "INPUT" || el.closest(TYPING)) return true;
  try {
    return el.matches(":focus-visible");
  } catch {
    return true; // no such selector here: assume the keyboard
  }
}

/** Should a dialog's Enter-to-confirm run for this key event? */
export function enterConfirms(e: KeyboardEvent): boolean {
  return e.key === "Enter" && !targetOwnsKey(e.target, "Enter");
}
