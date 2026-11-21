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
const ARROWS = ["[role=slider]", ...PICKERS].join(", ");
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

/** Should a dialog's Enter-to-confirm run for this key event? */
export function enterConfirms(e: KeyboardEvent): boolean {
  return e.key === "Enter" && !targetOwnsKey(e.target, "Enter");
}
