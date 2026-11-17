# Coordinates and units

PDF's bottom-left origin holds only for an unrotated page whose box starts at 0,0. The document layer uses the page as displayed (after `Rotate` and `CropBox`), in points, origin top-left; the object layer uses raw PDF user space (ISO 32000-2:2020, 8.3.2.3); public transforms between them account for rotation, crop offset and `UserUnit` (7.7.3.3). As displayed matches rendered images and reading order, and both spaces stay available; the price is that a document-layer coordinate holds only for the rotation and `CropBox` it was read under.
