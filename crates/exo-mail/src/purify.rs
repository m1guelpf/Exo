//! DOMPurify's default allowlists (DOMPurify 3, Apache-2.0 / MPL-2.0), which the sanitizer uses,
//! grouped as DOMPurify groups them. Names are lowercase.

use std::{collections::HashSet, sync::LazyLock};

/// A set of element or attribute names.
pub(crate) type Names = LazyLock<HashSet<&'static str>>;

const HTML: &str = "\
    a abbr acronym address area article aside audio b bdi bdo big blink blockquote body br \
    button canvas caption center cite code col colgroup content data datalist dd decorator del \
    details dfn dialog dir div dl dt element em fieldset figcaption figure font footer form h1 \
    h2 h3 h4 h5 h6 head header hgroup hr html i img input ins kbd label legend li main map \
    mark marquee menu menuitem meter nav nobr ol optgroup option output p picture pre progress \
    q rp rt ruby s samp search section select shadow slot small source spacer span strike \
    strong style sub summary sup table tbody td template textarea tfoot th thead time tr track \
    tt u ul var video wbr";
const SVG: &str = "\
    a altglyph altglyphdef altglyphitem animatecolor animatemotion animatetransform circle \
    clippath defs desc ellipse enterkeyhint exportparts filter font g glyph glyphref hkern \
    image inputmode line lineargradient marker mask metadata mpath part path pattern polygon \
    polyline radialgradient rect stop style svg switch symbol text textpath title tref tspan \
    view vkern";
const SVG_FILTERS: &str = "\
    feblend fecolormatrix fecomponenttransfer fecomposite feconvolvematrix fediffuselighting \
    fedisplacementmap fedistantlight fedropshadow feflood fefunca fefuncb fefuncg fefuncr \
    fegaussianblur feimage femerge femergenode femorphology feoffset fepointlight \
    fespecularlighting fespotlight fetile feturbulence";
const SVG_DISALLOWED: &str = "\
    animate color-profile cursor discard font-face font-face-format font-face-name \
    font-face-src font-face-uri foreignobject hatch hatchpath mesh meshgradient meshpatch \
    meshrow missing-glyph script set solidcolor unknown use";
const MATHML: &str = "\
    math menclose merror mfenced mfrac mglyph mi mlabeledtr mmultiscripts mn mo mover mpadded \
    mphantom mprescripts mroot mrow ms mspace msqrt mstyle msub msubsup msup mtable mtd mtext \
    mtr munder munderover";
const MATHML_DISALLOWED: &str = "\
    annotation annotation-xml maction maligngroup malignmark mlongdiv mprescripts mscarries \
    mscarry msgroup msline msrow mstack none semantics";

/// Elements kept.
pub(crate) static TAGS: Names = LazyLock::new(|| names(&[HTML, SVG, SVG_FILTERS, MATHML]));
/// Every SVG element, allowed or not (`ALL_SVG_TAGS`).
pub(crate) static SVG_TAGS: Names = LazyLock::new(|| names(&[SVG, SVG_FILTERS, SVG_DISALLOWED]));
/// Every MathML element, allowed or not.
pub(crate) static MATHML_TAGS: Names = LazyLock::new(|| names(&[MATHML, MATHML_DISALLOWED]));

/// Attributes kept (HTML, SVG, MathML and XML).
pub(crate) static ATTRIBUTES: Names = LazyLock::new(|| {
    names(&["\
        accent accent-height accentunder accept accumulate action additive align \
        alignment-baseline alt amplitude ascent attributename attributetype autocapitalize \
        autocomplete autopictureinpicture autoplay azimuth background basefrequency \
        baseline-shift begin bevelled bgcolor bias border by capture cellpadding cellspacing \
        checked cite class clear clip clip-path clip-rule clippathunits close color \
        color-interpolation color-interpolation-filters color-profile color-rendering cols \
        colspan columnalign columnlines columnspacing columnspan command commandfor controls \
        controlslist coords crossorigin cx cy d datetime decoding default denomalign depth \
        diffuseconstant dir direction disabled disablepictureinpicture disableremoteplayback \
        display displaystyle divisor dominant-baseline download draggable dur dx dy edgemode \
        elevation encoding enctype end enterkeyhint exponent exportparts face fence fill \
        fill-opacity fill-rule filter filterunits flood-color flood-opacity font-family \
        font-size font-size-adjust font-stretch font-style font-variant font-weight for frame \
        fx fy g1 g2 glyph-name glyphref gradienttransform gradientunits headers height hidden \
        high href hreflang id image-rendering in in2 inert inputmode integrity intercept ismap \
        k k1 k2 k3 k4 kernelmatrix kernelunitlength kerning keypoints keysplines keytimes kind \
        label lang largeop length lengthadjust letter-spacing lighting-color linethickness \
        list loading local loop low lquote lspace marker-end marker-mid marker-start \
        markerheight markerunits markerwidth mask mask-type maskcontentunits maskunits \
        mathbackground mathcolor mathsize mathvariant max maxlength maxsize media method min \
        minlength minsize mode movablelimits multiple muted name nonce noshade notation \
        novalidate nowrap numalign numoctaves offset opacity open operator optimum order \
        orient orientation origin overflow paint-order part path pathlength pattern \
        patterncontentunits patterntransform patternunits placeholder playsinline \
        pointer-events points popover popovertarget popovertargetaction poster preload \
        preservealpha preserveaspectratio primitiveunits pubdate r radiogroup radius readonly \
        refx refy rel repeatcount repeatdur required restart result rev reversed role rotate \
        rowalign rowlines rows rowspacing rowspan rquote rspace rx ry scale scope scriptlevel \
        scriptminsize scriptsizemultiplier seed selected selection separator separators shape \
        shape-rendering size sizes slope slot span specularconstant specularexponent \
        spellcheck spreadmethod src srclang srcset start startoffset stddeviation step \
        stitchtiles stop-color stop-opacity stretchy stroke stroke-dasharray stroke-dashoffset \
        stroke-linecap stroke-linejoin stroke-miterlimit stroke-opacity stroke-width style \
        subscriptshift summary supscriptshift surfacescale symmetric systemlanguage tabindex \
        tablevalues targetx targety text-anchor text-decoration text-orientation \
        text-rendering textlength title transform transform-origin translate type u1 u2 \
        unicode usemap valign value values vector-effect version vert-adv-y vert-origin-x \
        vert-origin-y viewbox visibility voffset width word-spacing wrap writing-mode x x1 x2 \
        xchannelselector xlink:href xlink:title xml:id xml:space xmlns xmlns:xlink y y1 y2 \
        ychannelselector z zoomandpan"])
});

/// Removed elements whose content goes with them (the rest are unwrapped).
pub(crate) static FORBID_CONTENTS: Names = LazyLock::new(|| {
    names(&["\
        annotation-xml audio colgroup desc foreignobject head iframe math mi mn mo ms mtext \
        noembed noframes noscript plaintext script selectedcontent style svg template thead \
        title video xmp"])
});

/// Attributes whose values are never checked as URLs.
pub(crate) static URI_SAFE_ATTRIBUTES: Names = LazyLock::new(|| {
    names(&["alt class for id label name pattern placeholder role style summary title value xmlns"])
});

/// Elements that may load `data:` URLs.
pub(crate) static DATA_URI_TAGS: Names =
    LazyLock::new(|| names(&["audio image img source track video"]));

fn names(lists: &[&'static str]) -> HashSet<&'static str> {
    lists
        .iter()
        .flat_map(|list| list.split_ascii_whitespace())
        .collect()
}
