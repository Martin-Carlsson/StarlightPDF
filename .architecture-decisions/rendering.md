# Rendering

## Fixed point

Floating point can round differently per platform and compiler. The renderer written here computes in fixed-point integers only. Every platform and binding then gives the same pixels, which one conformance suite can name; the price is maths of our own and a limited range.

## RGB blending and CMYK by formula

A page can blend in any colour space, and viewers convert DeviceCMYK through a profile of their choice. Rendering blends in RGB and converts DeviceCMYK by the formula of ISO 32000-2:2020, 10.4.2. No profile ships and every layer has one pixel format; the price is colours that differ from colour-managed viewers.

## Text without its font fails

Whether to bundle substitute fonts and predefined CMaps is undecided. Text in a font that is not embedded, or with a predefined CMap other than Identity, fails `Unsupported` where it would paint or clip; text that paints nothing does not. A blank or substituted page would be silently wrong, and scanned pages with an invisible text layer still render.

## A work limit

A small file can ask for endless drawing, and a callback protects only callers who pass one. Each render has a work limit, which grows with the decoded-data limit up to a ceiling, plus the pixels up to 25 megapixels; past it, `LimitExceeded`. Each pixel output takes work too, so a page asking for more pixels than the limit allows fails at once: a caller who wants it sets `fit` or `resolution`. Each band may redo the first pass's content work, at most the content part of the limit again in all. So up to 25 megapixels a higher resolution fails only where painting many coats per pixel, or content each band repeats, outgrows the limit. A unit is about 5 ns: each kind of work is charged so that its worst input measured costs at most 1.5 times that, a colour space or function by what one conversion through it costs. Untrusted input then ends: the worst hostile page measured took 22 s at 25 megapixels natively, and the worst through WebAssembly in Node 35 s. The price is that the most complex honest pages fail.
