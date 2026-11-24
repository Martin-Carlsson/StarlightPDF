//! Standard encodings and glyph names (ISO 32000-2:2020, Annex D; Technical Note 5176).

use crate::font_program::{Span, size};

/// Unicode of codes 128 to 159 of `WinAnsiEncoding`; 0 where it has a bullet for want of a character.
const CP1252: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC,
    0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

/// CFF's Expert charset, as runs of SIDs (first, count).
pub(crate) const EXPERT_CHARSET: [(u16, u16); 17] = [
    (0, 2),
    (229, 10),
    (13, 3),
    (99, 1),
    (239, 10),
    (27, 2),
    (249, 18),
    (109, 2),
    (267, 52),
    (158, 1),
    (155, 1),
    (163, 1),
    (319, 8),
    (150, 1),
    (164, 1),
    (169, 1),
    (327, 52),
];

/// CFF's Expert encoding, as runs (first code, count, first SID).
pub(crate) const EXPERT_ENCODING: [(u8, u8, u16); 31] = [
    (32, 1, 1),
    (33, 2, 229),
    (36, 8, 231),
    (44, 3, 13),
    (47, 1, 99),
    (48, 10, 239),
    (58, 2, 27),
    (60, 4, 249),
    (65, 5, 253),
    (73, 1, 258),
    (76, 4, 259),
    (82, 3, 263),
    (86, 1, 266),
    (87, 2, 109),
    (89, 3, 267),
    (93, 34, 270),
    (161, 3, 304),
    (166, 5, 307),
    (172, 1, 312),
    (175, 1, 313),
    (178, 2, 314),
    (182, 3, 316),
    (188, 1, 158),
    (189, 1, 155),
    (190, 1, 163),
    (191, 7, 319),
    (200, 1, 326),
    (201, 1, 150),
    (202, 1, 164),
    (203, 1, 169),
    (204, 52, 327),
];

/// CFF's Expert Subset charset, as runs of SIDs (first, count).
pub(crate) const EXPERT_SUBSET: [(u16, u16); 23] = [
    (0, 2),
    (231, 2),
    (235, 4),
    (13, 3),
    (99, 1),
    (239, 10),
    (27, 2),
    (249, 3),
    (253, 14),
    (109, 2),
    (267, 4),
    (272, 1),
    (300, 3),
    (305, 1),
    (314, 2),
    (158, 1),
    (155, 1),
    (163, 1),
    (320, 7),
    (150, 1),
    (164, 1),
    (169, 1),
    (327, 20),
];

/// CFF's `ISOAdobe` charset, as a run of SIDs.
pub(crate) const ISO_ADOBE: [(u16, u16); 1] = [(0, 229)];

/// `MacExpertEncoding` from code 32 (Table D.4); a hyphen where it has none.
const MAC_EXPERT: &str = "
space exclamsmall Hungarumlautsmall centoldstyle dollaroldstyle dollarsuperior ampersandsmall
Acutesmall parenleftsuperior parenrightsuperior twodotenleader onedotenleader comma hyphen
period fraction zerooldstyle oneoldstyle twooldstyle threeoldstyle fouroldstyle fiveoldstyle
sixoldstyle sevenoldstyle eightoldstyle nineoldstyle colon semicolon - threequartersemdash -
questionsmall - - - - Ethsmall - - onequarter onehalf threequarters oneeighth threeeighths
fiveeighths seveneighths onethird twothirds - - - - - - ff fi fl ffi ffl parenleftinferior -
parenrightinferior Circumflexsmall hypheninferior Gravesmall Asmall Bsmall Csmall Dsmall
Esmall Fsmall Gsmall Hsmall Ismall Jsmall Ksmall Lsmall Msmall Nsmall Osmall Psmall Qsmall
Rsmall Ssmall Tsmall Usmall Vsmall Wsmall Xsmall Ysmall Zsmall colonmonetary onefitted rupiah
Tildesmall - - asuperior centsuperior - - - - Aacutesmall Agravesmall Acircumflexsmall
Adieresissmall Atildesmall Aringsmall Ccedillasmall Eacutesmall Egravesmall Ecircumflexsmall
Edieresissmall Iacutesmall Igravesmall Icircumflexsmall Idieresissmall Ntildesmall Oacutesmall
Ogravesmall Ocircumflexsmall Odieresissmall Otildesmall Uacutesmall Ugravesmall
Ucircumflexsmall Udieresissmall - eightsuperior fourinferior threeinferior sixinferior
eightinferior seveninferior Scaronsmall - centinferior twoinferior - Dieresissmall -
Caronsmall osuperior fiveinferior - commainferior periodinferior Yacutesmall - dollarinferior
- - Thornsmall - nineinferior zeroinferior Zcaronsmall AEsmall Oslashsmall questiondownsmall
oneinferior Lslashsmall - - - - - - Cedillasmall - - - - - OEsmall figuredash hyphensuperior -
- - - exclamdownsmall - Ydieresissmall - onesuperior twosuperior threesuperior foursuperior
fivesuperior sixsuperior sevensuperior ninesuperior zerosuperior - esuperior rsuperior
tsuperior - - isuperior ssuperior dsuperior - - - - - lsuperior Ogoneksmall Brevesmall
Macronsmall bsuperior nsuperior msuperior commasuperior periodsuperior Dotaccentsmall
Ringsmall";

/// The last 32 of the 258 Macintosh glyph names of `post`.
const MAC_MORE: &str = "
Lslash lslash Scaron scaron Zcaron zcaron brokenbar Eth eth Yacute yacute Thorn thorn minus
multiply onesuperior twosuperior threesuperior onehalf onequarter threequarters franc Gbreve
gbreve Idotaccent Scedilla scedilla Cacute cacute Ccaron ccaron dcroat";

/// Mac OS Roman from code 128, the names 98 to 225 of `post`.
const MAC_OS: &str = "
Adieresis Aring Ccedilla Eacute Ntilde Odieresis Udieresis aacute agrave acircumflex adieresis
atilde aring ccedilla eacute egrave ecircumflex edieresis iacute igrave icircumflex idieresis
ntilde oacute ograve ocircumflex odieresis otilde uacute ugrave ucircumflex udieresis dagger
degree cent sterling section bullet paragraph germandbls registered copyright trademark acute
dieresis notequal AE Oslash infinity plusminus lessequal greaterequal yen mu partialdiff
summation product pi integral ordfeminine ordmasculine Omega ae oslash questiondown exclamdown
logicalnot radical florin approxequal Delta guillemotleft guillemotright ellipsis
nonbreakingspace Agrave Atilde Otilde OE oe endash emdash quotedblleft quotedblright quoteleft
quoteright divide lozenge ydieresis Ydieresis fraction currency guilsinglleft guilsinglright
fi fl daggerdbl periodcentered quotesinglbase quotedblbase perthousand Acircumflex Ecircumflex
Aacute Edieresis Egrave Iacute Icircumflex Idieresis Igrave Oacute Ocircumflex apple Ograve
Uacute Ucircumflex Ugrave dotlessi circumflex tilde macron breve dotaccent ring cedilla
hungarumlaut ogonek caron";

/// The codes Mac OS Roman has beyond `MacRomanEncoding` (9.6.5.4).
const MAC_OS_ONLY: [u8; 15] = [
    173, 176, 178, 179, 182, 183, 184, 185, 186, 189, 195, 197, 198, 215, 240,
];

/// Unicode of the names of Annex D that the code pages do not give.
const OTHERS: &str = "
fraction:2044 fraction:2215 fi:FB01 fl:FB02 ff:FB00 ffi:FB03 ffl:FB04 dotlessi:0131
Lslash:0141 lslash:0142 breve:02D8 caron:02C7 dotaccent:02D9 hungarumlaut:02DD ogonek:02DB
ring:02DA minus:2212 macron:02C9 periodcentered:2219 mu:03BC nonbreakingspace:00A0
notequal:2260 infinity:221E lessequal:2264 greaterequal:2265 partialdiff:2202 summation:2211
product:220F pi:03C0 integral:222B Omega:2126 Omega:03A9 radical:221A approxequal:2248
Delta:2206 Delta:0394 lozenge:25CA apple:F8FF Alpha:0391 Beta:0392 Gamma:0393 Epsilon:0395
Zeta:0396 Eta:0397 Theta:0398 Iota:0399 Kappa:039A Lambda:039B Mu:039C Nu:039D Xi:039E
Omicron:039F Pi:03A0 Rho:03A1 Sigma:03A3 Tau:03A4 Upsilon:03A5 Phi:03A6 Chi:03A7 Psi:03A8
alpha:03B1 beta:03B2 gamma:03B3 delta:03B4 epsilon:03B5 zeta:03B6 eta:03B7 theta:03B8
iota:03B9 kappa:03BA lambda:03BB nu:03BD xi:03BE omicron:03BF rho:03C1 sigma1:03C2 sigma:03C3
tau:03C4 upsilon:03C5 phi:03C6 chi:03C7 psi:03C8 omega:03C9 theta1:03D1 Upsilon1:03D2
phi1:03D5 omega1:03D6 universal:2200 existential:2203 suchthat:220B asteriskmath:2217
congruent:2245 therefore:2234 perpendicular:22A5 similar:223C minute:2032 second:2033
arrowboth:2194 arrowleft:2190 arrowup:2191 arrowright:2192 arrowdown:2193 carriagereturn:21B5
arrowdblboth:21D4 arrowdblleft:21D0 arrowdblup:21D1 arrowdblright:21D2 arrowdbldown:21D3
club:2663 diamond:2666 heart:2665 spade:2660 proportional:221D equivalence:2261 aleph:2135
Ifraktur:2111 Rfraktur:211C weierstrass:2118 circlemultiply:2297 circleplus:2295 emptyset:2205
intersection:2229 union:222A propersuperset:2283 reflexsuperset:2287 notsubset:2284
propersubset:2282 reflexsubset:2286 element:2208 notelement:2209 angle:2220 gradient:2207
dotmath:22C5 logicaland:2227 logicalor:2228 angleleft:2329 angleright:232A integraltp:2320
integralbt:2321 colonmonetary:20A1 onethird:2153 twothirds:2154 oneeighth:215B
threeeighths:215C fiveeighths:215D seveneighths:215E figuredash:2012 onedotenleader:2024
twodotenleader:2025";

/// `StandardEncoding`, as runs (first code, count, first SID).
pub(crate) const STANDARD_ENCODING: [(u8, u8, u16); 14] = [
    (32, 95, 1),
    (161, 15, 96),
    (177, 4, 111),
    (182, 8, 115),
    (191, 1, 123),
    (193, 8, 124),
    (202, 2, 132),
    (205, 4, 134),
    (225, 1, 138),
    (227, 1, 139),
    (232, 4, 140),
    (241, 1, 144),
    (245, 1, 145),
    (248, 4, 146),
];

/// The standard strings of CFF (TN 5176, Appendix A); 1 to 95 are ASCII's.
const STRINGS: &str = "
.notdef space exclam quotedbl numbersign dollar percent ampersand quoteright parenleft
parenright asterisk plus comma hyphen period slash zero one two three four five six seven
eight nine colon semicolon less equal greater question at A B C D E F G H I J K L M N O P Q R
S T U V W X Y Z bracketleft backslash bracketright asciicircum underscore quoteleft a b c d e
f g h i j k l m n o p q r s t u v w x y z braceleft bar braceright asciitilde exclamdown cent
sterling fraction yen florin section currency quotesingle quotedblleft guillemotleft
guilsinglleft guilsinglright fi fl endash dagger daggerdbl periodcentered paragraph bullet
quotesinglbase quotedblbase quotedblright guillemotright ellipsis perthousand questiondown
grave acute circumflex tilde macron breve dotaccent dieresis ring cedilla hungarumlaut ogonek
caron emdash AE ordfeminine Lslash Oslash OE ordmasculine ae dotlessi lslash oslash oe
germandbls onesuperior logicalnot mu trademark Eth onehalf plusminus Thorn onequarter divide
brokenbar degree thorn threequarters twosuperior registered minus eth multiply threesuperior
copyright Aacute Acircumflex Adieresis Agrave Aring Atilde Ccedilla Eacute Ecircumflex
Edieresis Egrave Iacute Icircumflex Idieresis Igrave Ntilde Oacute Ocircumflex Odieresis
Ograve Otilde Scaron Uacute Ucircumflex Udieresis Ugrave Yacute Ydieresis Zcaron aacute
acircumflex adieresis agrave aring atilde ccedilla eacute ecircumflex edieresis egrave iacute
icircumflex idieresis igrave ntilde oacute ocircumflex odieresis ograve otilde scaron uacute
ucircumflex udieresis ugrave yacute ydieresis zcaron exclamsmall Hungarumlautsmall
dollaroldstyle dollarsuperior ampersandsmall Acutesmall parenleftsuperior parenrightsuperior
twodotenleader onedotenleader zerooldstyle oneoldstyle twooldstyle threeoldstyle fouroldstyle
fiveoldstyle sixoldstyle sevenoldstyle eightoldstyle nineoldstyle commasuperior
threequartersemdash periodsuperior questionsmall asuperior bsuperior centsuperior dsuperior
esuperior isuperior lsuperior msuperior nsuperior osuperior rsuperior ssuperior tsuperior ff
ffi ffl parenleftinferior parenrightinferior Circumflexsmall hyphensuperior Gravesmall Asmall
Bsmall Csmall Dsmall Esmall Fsmall Gsmall Hsmall Ismall Jsmall Ksmall Lsmall Msmall Nsmall
Osmall Psmall Qsmall Rsmall Ssmall Tsmall Usmall Vsmall Wsmall Xsmall Ysmall Zsmall
colonmonetary onefitted rupiah Tildesmall exclamdownsmall centoldstyle Lslashsmall Scaronsmall
Zcaronsmall Dieresissmall Brevesmall Caronsmall Dotaccentsmall Macronsmall figuredash
hypheninferior Ogoneksmall Ringsmall Cedillasmall questiondownsmall oneeighth threeeighths
fiveeighths seveneighths onethird twothirds zerosuperior foursuperior fivesuperior sixsuperior
sevensuperior eightsuperior ninesuperior zeroinferior oneinferior twoinferior threeinferior
fourinferior fiveinferior sixinferior seveninferior eightinferior nineinferior centinferior
dollarinferior periodinferior commainferior Agravesmall Aacutesmall Acircumflexsmall
Atildesmall Adieresissmall Aringsmall AEsmall Ccedillasmall Egravesmall Eacutesmall
Ecircumflexsmall Edieresissmall Igravesmall Iacutesmall Icircumflexsmall Idieresissmall
Ethsmall Ntildesmall Ogravesmall Oacutesmall Ocircumflexsmall Otildesmall Odieresissmall
OEsmall Oslashsmall Ugravesmall Uacutesmall Ucircumflexsmall Udieresissmall Yacutesmall
Thornsmall Ydieresissmall 001.000 001.001 001.002 001.003 Black Bold Book Light Medium Regular
Roman Semibold";

/// `WinAnsiEncoding` from code 127 (Table D.2).
const WIN_ANSI: &str = "
bullet Euro bullet quotesinglbase florin quotedblbase ellipsis dagger daggerdbl circumflex
perthousand Scaron guilsinglleft OE bullet Zcaron bullet bullet quoteleft quoteright
quotedblleft quotedblright bullet endash emdash tilde trademark scaron guilsinglright oe
bullet zcaron Ydieresis space exclamdown cent sterling currency yen brokenbar section dieresis
copyright ordfeminine guillemotleft logicalnot hyphen registered macron degree plusminus
twosuperior threesuperior acute mu paragraph periodcentered cedilla onesuperior ordmasculine
guillemotright onequarter onehalf threequarters questiondown Agrave Aacute Acircumflex Atilde
Adieresis Aring AE Ccedilla Egrave Eacute Ecircumflex Edieresis Igrave Iacute Icircumflex
Idieresis Eth Ntilde Ograve Oacute Ocircumflex Otilde Odieresis multiply Oslash Ugrave Uacute
Ucircumflex Udieresis Yacute Thorn germandbls agrave aacute acircumflex atilde adieresis aring
ae ccedilla egrave eacute ecircumflex edieresis igrave iacute icircumflex idieresis eth ntilde
ograve oacute ocircumflex otilde odieresis divide oslash ugrave uacute ucircumflex udieresis
yacute thorn ydieresis";

/// A predefined encoding (Annex D).
#[derive(Clone, Copy)]
pub(crate) enum Base {
    /// `MacExpertEncoding`.
    MacExpert,
    /// `MacRomanEncoding`.
    MacRoman,
    /// `StandardEncoding`.
    Standard,
    /// `WinAnsiEncoding`.
    WinAnsi,
}

/// Where a glyph's name is.
#[derive(Clone, Copy)]
pub(crate) enum Label {
    /// In the program: position and length.
    Held(u32, u8),
    /// In a standard list.
    Listed(u16),
}

/// Glyphs by name.
#[derive(Default)]
pub(crate) struct Names {
    /// Each name and its glyph, sorted by name.
    entries: Vec<(Label, u16)>,
    /// The standard list.
    list: Vec<&'static [u8]>,
    /// The work sorting took: its comparisons, each of the longest name.
    work: u64,
}

impl Base {
    /// The name of `code`; `None` where the encoding has none.
    pub(crate) fn glyph_name(self, code: u8) -> Option<&'static [u8]> {
        let name = match self {
            Self::MacExpert => word(MAC_EXPERT, code.checked_sub(32)?).filter(|name| *name != "-"),
            Self::MacRoman if MAC_OS_ONLY.contains(&code) => None,
            Self::MacRoman if code == 202 => Some("space"),
            Self::MacRoman => code
                .checked_sub(128)
                .map_or_else(|| ascii(code), |index| word(MAC_OS, index)),
            Self::Standard => STRINGS
                .split_ascii_whitespace()
                .nth(usize::from(listed(&STANDARD_ENCODING, code)?)),
            Self::WinAnsi => code
                .checked_sub(127)
                .map_or_else(|| ascii(code), |index| word(WIN_ANSI, index)),
        };
        name.map(str::as_bytes)
    }

    /// The encoding called `name`.
    pub(crate) const fn named(name: &[u8]) -> Option<Self> {
        match name {
            b"MacExpertEncoding" => Some(Self::MacExpert),
            b"MacRomanEncoding" => Some(Self::MacRoman),
            b"StandardEncoding" => Some(Self::Standard),
            b"WinAnsiEncoding" => Some(Self::WinAnsi),
            _ => None,
        }
    }
}

impl Label {
    /// The name in `span`; `None` over 255 bytes, which no name has.
    pub(crate) fn held(span: Span) -> Option<Self> {
        let length = u8::try_from(span.length).ok()?;
        Some(Self::Held(span.at, length))
    }

    /// Its bytes.
    fn text<'font>(self, bytes: &'font [u8], list: &[&'static [u8]]) -> &'font [u8] {
        let found = match self {
            Self::Held(at, length) => Span {
                at,
                length: u32::from(length),
            }
            .of(bytes),
            Self::Listed(index) => list.get(usize::from(index)).copied(),
        };
        found.unwrap_or_default()
    }
}

impl Names {
    /// The first glyph called `name`.
    pub(crate) fn find(&self, bytes: &[u8], name: &[u8]) -> Option<u32> {
        let text = |entry: &(Label, u16)| entry.0.text(bytes, &self.list);
        let at = self.entries.partition_point(|entry| text(entry) < name);
        let entry = self.entries.get(at)?;
        (text(entry) == name).then(|| u32::from(entry.1))
    }

    /// The bytes it holds.
    pub(crate) fn held(&self) -> u64 {
        let entries = self
            .entries
            .capacity()
            .saturating_mul(size_of::<(Label, u16)>());
        let list = self.list.capacity().saturating_mul(size_of::<&[u8]>());
        u64::try_from(entries.saturating_add(list)).unwrap_or(u64::MAX)
    }

    /// Sorts `entries`, whose names lie in `bytes` or `list`.
    pub(crate) fn new(
        bytes: &[u8],
        list: Vec<&'static [u8]>,
        mut entries: Vec<(Label, u16)>,
    ) -> Self {
        let longest = entries
            .iter()
            .map(|entry| entry.0.text(bytes, &list).len())
            .max()
            .unwrap_or(0);
        let count = u64::try_from(entries.len()).unwrap_or(u64::MAX);
        // About n log n comparisons, a unit for each 16 bytes compared.
        let work = count
            .saturating_mul(u64::from(count.max(1).ilog2().saturating_add(1)))
            .saturating_mul(
                u64::try_from(longest >> 4_u32)
                    .unwrap_or(u64::MAX)
                    .saturating_add(1),
            );
        // Stable, so the first of equal names wins.
        entries.sort_by(|left, right| left.0.text(bytes, &list).cmp(right.0.text(bytes, &list)));
        Self {
            entries,
            list,
            work,
        }
    }

    /// The work sorting took.
    pub(crate) const fn work(&self) -> u64 {
        self.work
    }
}

/// The name of an ASCII `code` in `WinAnsiEncoding` and `MacRomanEncoding`.
fn ascii(code: u8) -> Option<&'static str> {
    match code {
        39 => Some("quotesingle"),
        96 => Some("grave"),
        32..=126 => word(STRINGS, code.checked_sub(31)?),
        _ => None,
    }
}

/// The ASCII code of `name`.
fn ascii_code(name: &[u8]) -> Option<u8> {
    match name {
        b"quotesingle" => Some(39),
        b"grave" => Some(96),
        b"quoteleft" | b"quoteright" => None,
        _ => STRINGS
            .split_ascii_whitespace()
            .zip(31_u8..=126)
            .skip(1)
            .find_map(|(word, code)| (word.as_bytes() == name).then_some(code)),
    }
}

/// The characters `name` may stand for (9.6.5.4): `uniXXXX`, `uXXXX`, or Annex D's.
pub(crate) fn characters(name: &[u8]) -> Vec<u32> {
    let stem = name.split(|byte| *byte == b'.').next().unwrap_or_default();
    let fixed = stem.strip_prefix(b"uni").filter(|digits| digits.len() == 4);
    let free = || {
        stem.strip_prefix(b"u")
            .filter(|digits| (4..=6).contains(&digits.len()))
    };
    if let Some(value) = fixed.or_else(free).and_then(hexadecimal) {
        return vec![value];
    }
    let mut found: Vec<u32> = ascii_code(stem).map(u32::from).into_iter().collect();
    for (word, code) in WIN_ANSI.split_ascii_whitespace().zip(127_u32..) {
        if word.as_bytes() != stem {
            continue;
        }
        let character = match code {
            0..128 => None,
            128..160 => CP1252
                .get(size(code.saturating_sub(128)))
                .copied()
                .filter(|value| *value != 0)
                .map(u32::from),
            _ => Some(code),
        };
        found.extend(character);
    }
    for (word, value) in OTHERS
        .split_ascii_whitespace()
        .filter_map(|entry| entry.split_once(':'))
    {
        if word.as_bytes() == stem {
            found.extend(hexadecimal(value.as_bytes()));
        }
    }
    found
}

/// The number the hexadecimal `digits` make.
fn hexadecimal(digits: &[u8]) -> Option<u32> {
    u32::from_str_radix(str::from_utf8(digits).ok()?, 16)
        .ok()
        .filter(|_value| digits.iter().all(u8::is_ascii_hexdigit))
}

/// The SID at `code` in `runs`.
pub(crate) fn listed(runs: &[(u8, u8, u16)], code: u8) -> Option<u16> {
    runs.iter().find_map(|run| {
        let (first, count, id) = *run;
        let place = code.checked_sub(first).filter(|place| *place < count)?;
        id.checked_add(u16::from(place))
    })
}

/// The 258 Macintosh glyph names of `post`.
pub(crate) fn macintosh_glyphs() -> Vec<&'static [u8]> {
    let mut names = vec![".notdef", ".null", "nonmarkingreturn"];
    names.extend((32..=126).filter_map(ascii));
    names.extend(MAC_OS.split_ascii_whitespace());
    names.extend(MAC_MORE.split_ascii_whitespace());
    names.into_iter().map(str::as_bytes).collect()
}

/// The code of `name` in Mac OS Roman.
pub(crate) fn roman_code(name: &[u8]) -> Option<u8> {
    ascii_code(name).or_else(|| {
        MAC_OS
            .split_ascii_whitespace()
            .zip(128_u8..=u8::MAX)
            .find_map(|(word, code)| (word.as_bytes() == name).then_some(code))
    })
}

/// The bytes one look-up by name scans at most.
pub(crate) fn scanned() -> u64 {
    let lists = [STRINGS, WIN_ANSI, OTHERS, MAC_OS, MAC_EXPERT];
    let bytes = lists
        .iter()
        .fold(0_usize, |sum, list| sum.saturating_add(list.len()));
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// The standard strings of CFF.
pub(crate) fn standard_strings() -> Vec<&'static [u8]> {
    STRINGS
        .split_ascii_whitespace()
        .map(str::as_bytes)
        .collect()
}

/// The word `index` of `list`.
fn word(list: &'static str, index: u8) -> Option<&'static str> {
    list.split_ascii_whitespace().nth(usize::from(index))
}
