import React from "react";

interface LoginViewProps {
  stageRef: React.RefObject<HTMLDivElement>;
  frameRef: React.RefObject<HTMLDivElement>;
  frameStyle: React.CSSProperties;
}

export default function LoginView({ stageRef, frameRef, frameStyle }: LoginViewProps) {
  return (
    <div className="game-stage" ref={stageRef}>
      <div className="stage-grid" />
      <div className="stage-inner">
        <div className="aspect-frame" ref={frameRef} style={frameStyle}>
          Loading login page...
        </div>
      </div>
    </div>
  );
}
