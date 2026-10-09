import { ScanSearch } from "lucide-react";
import { plural } from "../../format";
import { HEAL_SIZE, HEAL_STEPS, MAX_SPOTS, sizeForRadius } from "../../heal";
import { useStore } from "../../store";
import { Slider } from "./Slider";

/**
 * Heal: covering dust, hairs and scratches. The size of the next spot (or
 * the chosen one), looking for dust, and the spots the photo has.
 */
export function HealPanel() {
  const spots = useStore((s) => s.editor.adjustments.spots);
  const spotIndex = useStore((s) => s.editor.spotIndex);
  const dust = useStore((s) => s.editor.dust);
  const healing = useStore((s) => s.editor.healing);
  const size = useStore((s) => s.healSize);
  const ready = useStore((s) => s.editor.ready && s.editor.photoId === s.openId);
  const s = useStore.getState();
  const chosen = spotIndex !== null ? spots[spotIndex] : undefined;
  const full = spots.length >= MAX_SPOTS;

  return (
    <div className={`crop-panel heal-panel ${ready ? "" : "waiting"}`} inert={!ready}>
      <section className="crop-group">
        <p className="panel-hint">
          {full
            ? `A photo can have ${MAX_SPOTS} spots. Delete some to heal more.`
            : "Click a speck to cover it, or drag over a hair or scratch. Its texture comes from a patch nearby; drag the patch somewhere better if it shows."}
        </p>
        <Slider
          label={chosen ? "Spot size" : "Size"}
          value={chosen ? sizeForRadius(chosen.radius) : size}
          min={1}
          max={100}
          origin={HEAL_SIZE}
          format={String}
          onChange={(value) => s.setHealSize(value)}
          commitLabel={() => HEAL_STEPS.resized}
        />
        {chosen && spotIndex !== null && (
          <div className="part-footer">
            <p className="panel-hint">[ and ] change its size.</p>
            <button className="button quiet" title="Delete the spot (Delete)" onClick={() => s.deleteSpot(spotIndex)}>
              Delete spot
            </button>
          </div>
        )}
      </section>

      <section className="crop-group">
        <h3 className="crop-heading">Dust</h3>
        {dust ? (
          <>
            <p className="panel-hint">
              {plural(dust.length, "speck")} marked. Click one to remove it, or Alt-click one that isn’t dust to leave it.
            </p>
            <div className="part-footer">
              <button className="button quiet" onClick={() => s.dismissSpeck()}>
                Leave all
              </button>
              <button className="button primary" disabled={full} onClick={() => s.removeSpecks("all")}>
                Remove all
              </button>
            </div>
          </>
        ) : (
          <>
            <p className="panel-hint">Find the specks that stand out from their surroundings, to remove them all at once or one by one.</p>
            <button className="button find-dust" disabled={healing === "dust" || full} aria-busy={healing === "dust"} onClick={() => void s.findDust()}>
              {healing === "dust" ? <span className="spinner" /> : <ScanSearch size={15} />}
              {healing === "dust" ? "Looking for dust…" : "Find dust"}
            </button>
          </>
        )}
      </section>

      <div className="crop-actions heal-actions">
        <span className="crop-size">{spots.length > 0 ? plural(spots.length, "spot") : "No spots yet"}</span>
        <button className="button quiet" disabled={spots.length === 0} onClick={() => s.deleteAllSpots()}>
          Delete all
        </button>
      </div>
    </div>
  );
}
