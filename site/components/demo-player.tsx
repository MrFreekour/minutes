"use client";

import { Player } from "@remotion/player";
import { MinutesDemo } from "./minutes-demo";

export function DemoPlayer() {
  return (
    <div className="mx-auto w-full max-w-[720px] overflow-hidden rounded-[8px] border border-[color:var(--border)] text-left shadow-[var(--shadow-panel)]" style={{ maxHeight: "min(55vw, 380px)" }}>
      <Player
        component={MinutesDemo}
        durationInFrames={630}
        fps={15}
        compositionWidth={900}
        compositionHeight={550}
        style={{ width: "100%" }}
        autoPlay
        loop
        // Keeps the demo animating. MinutesDemo is silent, but an unmuted
        // Player still builds an AudioContext and drives its timeline off that
        // clock. Chrome leaves the context suspended until a user gesture, so
        // the Player reported isPlaying() === true while the clock never ticked
        // and the frame sat at 0: a demo that looked like a broken screenshot.
        //
        // SharedPlayerContext only creates the context when
        // `audioEnabled && !playerMuted && mediaVolume > 0`, so muting from the
        // first render skips it and playback runs on the normal RAF clock.
        // It must be `initiallyMuted`; the `muted` prop does not set
        // `playerMuted` and leaves the context in place.
        initiallyMuted
        acknowledgeRemotionLicense
      />
    </div>
  );
}
