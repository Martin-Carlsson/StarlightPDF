//! Renders arbitrary bytes as content: the page's, a form's, a pattern cell's and a soft mask's, each once with
//! the resources that let it reach itself and once without.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

/// An embedded CFF program of one box.
const FONT: &[u8] = include_bytes!("../../tests/corpus/render/font-cff.cff");

/// What the content can name.
const RESOURCES: &str = "/XObject << /X 5 0 R /I 8 0 R /Y 11 0 R >> /Pattern << /P 6 0 R /Q 12 0 R >> /Font << /F 7 0 R >> \
     /ExtGState << /G << /ca 0.5 /BM /Multiply >> /S << /SMask << /S /Alpha /G 5 0 R >> >> \
     /L << /SMask << /S /Luminosity /G 11 0 R /BC [0.5] \
     /TR << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> >> >> /A << /SMask << /S /Alpha /G 11 0 R >> >> /N << /SMask /None /AIS true /TK false >> >> \
     /Properties << /V << /Type /OCMD /OCGs [13 0 R] /VE [/Not 13 0 R] >> >> \
     /ColorSpace << /C [/Indexed /DeviceRGB 1 <FF0000 0000FF>] /U [/Pattern /DeviceRGB] >> \
     /Shading << /S << /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] \
     /Function << /FunctionType 2 /Domain [0 1] /N 1 >> >> >>";

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let resources = format!("/Resources << {RESOURCES} >>");
    drop(render(
        data,
        RESOURCES,
        vec![
            stream(
                &format!(
                    "/Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >> {resources}"
                ),
                data,
            ),
            stream(
                &format!(
                    "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 20 20] /XStep 20 /YStep 20 {resources}"
                ),
                data,
            ),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /SLTest /FirstChar 65 /LastChar 65 /Widths [1000] \
              /FontDescriptor 9 0 R >>"
                .to_vec(),
            stream(
                "/Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
                b"\x80",
            ),
            b"<< /Type /FontDescriptor /FontName /SLTest /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 \
              /Ascent 800 /Descent 0 /CapHeight 800 /StemV 80 /FontFile3 10 0 R >>"
                .to_vec(),
            stream("/Subtype /Type1C", FONT),
            stream(
                "/Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /K true /CS /DeviceGray >> /Resources << >>",
                data,
            ),
            stream(
                "/PatternType 1 /PaintType 2 /TilingType 2 /BBox [0 0 90 90] /XStep 70 /YStep 80 /Resources << >>",
                data,
            ),
            b"<< /Type /OCG /Name (V) >>".to_vec(),
        ],
    ));
});
