# Annotation color

A redaction mark has two colours, C for its outline and IC for its area (ISO 32000-2:2020, 12.5.6.23), but an annotation's options and info hold one `color`. For a redaction mark `color` is IC, the colour applying it fills its area with; C is not exposed. The fill is what stays in the file after redaction, and one field keeps every annotation kind and language alike; exposing C later needs a new field, not a change to `color`.
