# Last Train to the Moon

This directory contains the editable source assets for the Vifu short-drama
example. The project targets a 9:16 mobile frame and keeps generated media next
to the exported game source so the example can be reproduced outside the local
runtime volume.

## Layout

- `game-source.vifu.json`: exported Vifu runtime graph.
- `assets/images/backgrounds/`: portrait scene backgrounds.
- `assets/images/characters/`: character poses and expression variants.
- `assets/audio/music/`: score and ambience.
- `assets/audio/voices/`: editable dialogue voice clips.

The runtime currently imports the game source and media separately. The files in
this directory are the canonical inputs for the example and are ready for a
future single-project import/export bundle.
