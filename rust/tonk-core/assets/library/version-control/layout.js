(self, model) => {
  // Newest first, every revision above the revisions it built on: an
  // edition is one more than its deepest parent's, so sorting by edition
  // is a topological order. Ties are concurrent revisions.
  const order = [...model.revisions.values()].sort(
    (a, b) => b.edition - a.edition || (a.id < b.id ? -1 : 1),
  );
  // Each lane holds the revision it is waiting to reach, or null when free.
  const lanes = [];
  const claim = () => {
    const free = lanes.indexOf(null);
    if (free >= 0) return free;
    lanes.push(null);
    return lanes.length - 1;
  };
  const rows = [];
  let width = 1;
  for (const revision of order) {
    const before = lanes.slice();
    let lane = lanes.indexOf(revision.id);
    if (lane < 0) lane = claim();
    // Every other lane waiting on this revision converges on it.
    const converging = [];
    for (let index = 0; index < lanes.length; index++) {
      if (lanes[index] === revision.id) {
        if (index !== lane) converging.push(index);
        lanes[index] = null;
      }
    }
    // The deepest parent continues this lane; the others branch off.
    const outgoing = revision.parents.map((parent, index) => {
      let target = lanes.indexOf(parent);
      if (target < 0) {
        target = index === 0 && lanes[lane] === null ? lane : claim();
        lanes[target] = parent;
      }
      return target;
    });
    while (lanes.length && lanes[lanes.length - 1] === null) lanes.pop();
    const after = lanes.slice();
    width = Math.max(width, before.length, after.length, lane + 1);
    rows.push({ revision, lane, before, after, converging, outgoing });
  }
  return { rows, width };
}
