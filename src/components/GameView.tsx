import React from "react";

interface GameViewProps {
  stageRef: React.RefObject<HTMLDivElement>;
  frameRef: React.RefObject<HTMLDivElement>;
  frameStyle: React.CSSProperties;
}

export default function GameView({ stageRef, frameRef, frameStyle }: GameViewProps) {
  return (
    <div className="game-stage" ref={stageRef}>
      <div className="stage-grid" />
      <div className="stage-inner">
        <div className="aspect-frame" ref={frameRef} style={frameStyle}>
          Game viewport 960x560
        </div>
      </div>
    </div>
  );
}
