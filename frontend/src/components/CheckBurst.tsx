import React from 'react';

const ANGLES = [0, 60, 120, 180, 240, 300];

/** A 200 ms ring and six dots over a button that just confirmed something. Give it a new `n` per press; 0 draws nothing. */
export const CheckBurst: React.FC<{ n: number }> = ({ n }) =>
  n > 0 ? (
    <span key={n} className="kws-burst" aria-hidden="true" data-check-burst="">
      <span className="kws-burst-ring" />
      {ANGLES.map((a) => (
        <span key={a} className="kws-burst-dot" style={{ ['--kws-a' as string]: `${a}deg` }} />
      ))}
    </span>
  ) : null;
