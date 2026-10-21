// A TextEditingController that paints inline marks (bold, italic,
// underline, strikethrough, inline code, links, page and date mentions,
// text colours) from a list of ranges (UTF-16 offsets). Backgrounds (inline
// code, background colours) are not text styles: [MarkPainter] draws them
// as rounded rectangles behind the text. It also draws a page mention's
// icon in front of it, in a gap left by letter spacing on the character
// before the mention (the text itself stays the page title, so offsets
// do not change).

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../core/page_link.dart';
import '../registry/addon_registry.dart' show AddonInlineMark;
import '../ui/block_colors.dart';
import '../ui/icons.dart';
import '../ui/tokens.dart';
import '../ui/database/db_theme.dart';
import 'todo_dates.dart';

class RichController extends TextEditingController {
  RichController({super.text, List<BMark>? marks}) : _marks = marks ?? [];

  List<BMark> _marks;
  List<BMark> get marks => _marks;
  set marks(List<BMark> value) {
    _marks = value;
    if (inlineContext != null) syncInline();
  }
  AppColors colors = AppColors.light;

  /// Room left in front of page mentions for their icon (0: none).
  double mentionGap = 0;

  /// Resting to-do dates paint as chips. Focused text retains canonical
  /// characters so text editing, selection and the IME keep exact offsets.
  bool dateChips = false;
  bool todoChecked = false;

  /// The icon (emoji) of a mentioned page, or null for the default.
  String? Function(String pageId)? mentionIcon;

  /// The target's top-level space fill, never the containing page's colour.
  Color? Function(String pageId)? pageLinkFill;

  /// The add-ons that draw inline marks here (set by the row that shows the
  /// text). A mark of a kind none of them draws stays plain text.
  Iterable<AddonInlineMark> inlineMarks = const [];

  /// The context the marks' states are made in, while a row shows the text.
  /// Null means no states are made or kept.
  BuildContext? inlineContext;

  final _inline = <String, ChangeNotifier>{};

  AddonInlineMark? _providerOf(BMark mark) {
    for (final p in inlineMarks) {
      if (p.kind == mark.kind) return p;
    }
    return null;
  }

  String _inlineKey(BMark mark) => '${mark.kind.name}\n${mark.url}';

  /// The provider's live label for [mark] (for a query chip, "4 open #work
  /// tasks"), or null while it is unknown: the covered text shows instead.
  String? inlineLabelOf(BMark mark) {
    final state = _inline[_inlineKey(mark)];
    final provider = _providerOf(mark);
    if (state == null || provider == null) return null;
    return provider.label(state);
  }

  /// The live state of [mark]'s chip, or null when no add-on draws it.
  ChangeNotifier? inlineStateOf(BMark mark) => _inline[_inlineKey(mark)];

  /// The add-on that draws [mark], or null.
  AddonInlineMark? inlineProviderOf(BMark mark) => _providerOf(mark);

  /// Makes the state of each chip that has none, and disposes the states of
  /// marks that are gone. Runs when the marks change and while a row shows
  /// the text.
  void syncInline() {
    final context = inlineContext;
    final keep = <String>{};
    for (final mark in _marks) {
      final provider = _providerOf(mark);
      if (provider == null) continue;
      final key = _inlineKey(mark);
      keep.add(key);
      if (context == null || _inline.containsKey(key)) continue;
      final state = provider.watch(context, mark.url);
      state.addListener(refresh);
      _inline[key] = state;
    }
    for (final key in _inline.keys.where((k) => !keep.contains(k)).toList()) {
      _release(key);
    }
  }

  void _release(String key) {
    final state = _inline.remove(key)!;
    state.removeListener(refresh);
    state.dispose();
  }

  /// Disposes every chip state. The row that shows the text calls this when
  /// it goes away; its next build makes them again.
  void releaseInline() {
    inlineContext = null;
    for (final key in _inline.keys.toList()) {
      _release(key);
    }
  }

  @override
  void dispose() {
    releaseInline();
    super.dispose();
  }

  /// Repaints after the marks changed without a text change.
  void refresh() => notifyListeners();

  @override
  TextSpan buildTextSpan({
    required BuildContext context,
    TextStyle? style,
    required bool withComposing,
  }) {
    final t = text;
    final composing = value.composing;
    final hasComposing =
        withComposing &&
        composing.isValid &&
        !composing.isCollapsed &&
        composing.end <= t.length;
    return markedSpan(
      t,
      marks,
      colors,
      style,
      composing: hasComposing ? composing : null,
      mentionGap: mentionGap,
      dateChips: dateChips,
      inlineLabel: inlineLabelOf,
      textScaler: MediaQuery.textScalerOf(context),
    );
  }
}

/// [t] with [marks] as styled spans (also used for read-only text such as
/// the table of contents and version previews). With [mentionGap], the
/// character before each page mention gets that much letter spacing (room
/// for the icon [MarkPainter] draws).
TextSpan markedSpan(
  String t,
  List<BMark> marks,
  AppColors c,
  TextStyle? style, {
  TextRange? composing,
  double mentionGap = 0,
  bool dateChips = false,
  String? Function(BMark mark)? inlineLabel,
  TextScaler textScaler = TextScaler.noScaling,
}) {
  if (marks.isEmpty && composing == null) {
    return TextSpan(style: style, text: t);
  }
  final cuts = <int>{0, t.length};
  for (final m in marks) {
    cuts.add(m.start.clamp(0, t.length));
    cuts.add(m.end.clamp(0, t.length));
    if (dateChips &&
        m.kind == BMarkKind.date &&
        todoDate(m.url) != null &&
        m.start >= 0 &&
        m.end <= t.length &&
        m.start < m.end) {
      // Keep one invisible character at body size to preserve the line's
      // ascent, descent and height, even when the date is its only text.
      cuts.add(_datePrefixEnd(t, m.start, m.end));
    }
  }
  // Mention starts with a character before them: that character is a span
  // of its own (both halves of a surrogate pair).
  final gapAt = <int>{};
  if (mentionGap > 0) {
    for (final m in marks) {
      if (m.kind != BMarkKind.pageLink ||
          m.start <= 0 ||
          m.start > t.length ||
          m.start >= m.end) {
        continue;
      }
      gapAt.add(m.start);
      cuts.add(mentionGapChar(t, m.start));
    }
  }
  if (composing != null) {
    cuts
      ..add(composing.start)
      ..add(composing.end);
  }
  final points = cuts.toList()..sort();
  final spans = <InlineSpan>[];
  for (var i = 0; i + 1 < points.length; i++) {
    final a = points[i], b = points[i + 1];
    if (a >= b) continue;
    var bold = false,
        italic = false,
        strike = false,
        code = false,
        link = false,
        mention = false,
        under = false;
    String? date;
    BMark? queryMark;
    Color? color;
    for (final m in marks) {
      if (m.start <= a && m.end >= b) {
        switch (m.kind) {
          case BMarkKind.bold:
            bold = true;
          case BMarkKind.italic:
            italic = true;
          case BMarkKind.strike:
            strike = true;
          case BMarkKind.code:
            code = true;
          case BMarkKind.link:
            link = true;
          case BMarkKind.pageLink:
            mention = true;
          case BMarkKind.underline:
            under = true;
          case BMarkKind.color:
            color = paletteText(m.url, c) ?? color;
          case BMarkKind.date:
            date = m.url;
          case BMarkKind.query:
            queryMark = m;
        }
      }
    }
    final composingHere =
        composing != null && a >= composing.start && b <= composing.end;
    TextStyle? s;
    if (bold ||
        italic ||
        strike ||
        code ||
        link ||
        mention ||
        under ||
        composingHere ||
        color != null ||
        date != null) {
      final decos = <TextDecoration>[
        if (strike ||
            (style?.decoration?.contains(TextDecoration.lineThrough) ?? false))
          TextDecoration.lineThrough,
        if (link || mention || under || composingHere) TextDecoration.underline,
      ];
      s = TextStyle(
        fontWeight: bold ? FontWeight.w600 : (mention ? FontWeight.w500 : null),
        fontStyle: italic ? FontStyle.italic : null,
        decoration: decos.isEmpty ? null : TextDecoration.combine(decos),
        decorationColor: link
            ? c.accentText
            : (mention ? c.textSecondary.withValues(alpha: 0.6) : null),
        decorationThickness: link || mention ? 1 : null,
        color: code
            ? c.inlineCodeText
            : (color ??
                  (link
                      ? c.accentText
                      : (date != null ? c.textSecondary : null))),
        fontFamily: code ? monoFamily : null,
        fontFamilyFallback: code ? monoFallback : null,
        fontSize: code && style?.fontSize != null
            ? style!.fontSize! * 0.85
            : null,
      );
    }
    if (gapAt.contains(b) && a == mentionGapChar(t, b)) {
      s = (s ?? const TextStyle()).copyWith(
        letterSpacing: (style?.letterSpacing ?? 0) + mentionGap,
      );
    }
    var segment = t.substring(a, b);
    String? semantics;
    if (queryMark != null && inlineLabel != null) {
      final label = inlineLabel(queryMark);
      final mark = queryMark;
      if (label != null &&
          mark.start >= 0 &&
          mark.end <= t.length &&
          mark.start < mark.end) {
        // The chip's covered text is its fallback. It is drawn transparent
        // and spread so its width is the chip's: [MarkPainter] paints the
        // live label over the space.
        final base = style ?? const TextStyle(fontSize: 16);
        final covered = t.substring(mark.start, mark.end);
        final spare =
            queryChipWidth(label, base, textScaler) -
            _textWidth(covered.replaceAll(' ', ' '), base, textScaler);
        s = base.copyWith(
          color: const Color(0x00000000),
          decoration: TextDecoration.none,
          letterSpacing:
              (base.letterSpacing ?? 0) + spare / covered.runes.length,
        );
        segment = segment.replaceAll(' ', ' ');
        semantics = a == mark.start ? '$label search results' : '';
      }
    }
    if (dateChips && date != null && todoDate(date) != null) {
      final mark = marks.firstWhere(
        (m) => m.kind == BMarkKind.date && m.start <= a && m.end >= b,
      );
      if (mark.start >= 0 && mark.end <= t.length && mark.start < mark.end) {
        final label = todoDateLabel(todoDate(date)!);
        final prefixEnd = _datePrefixEnd(t, mark.start, mark.end);
        final prefix = t.substring(mark.start, prefixEnd);
        final tail = t.substring(prefixEnd, mark.end).replaceAll(' ', '\u00a0');
        final base = style ?? const TextStyle(fontSize: 16);
        final prefixPainter = TextPainter(
          text: TextSpan(text: prefix, style: base),
          textDirection: TextDirection.ltr,
          textScaler: textScaler,
        )..layout();
        final tailPainter = TextPainter(
          text: TextSpan(text: tail, style: base),
          textDirection: TextDirection.ltr,
          textScaler: textScaler,
        )..layout();
        final chip = TextPainter(
          text: TextSpan(
            text: label,
            style: _chipTextStyle(base, c.textSecondary),
          ),
          textDirection: TextDirection.ltr,
          textScaler: textScaler,
        )..layout();
        // Padding remains in logical pixels, just like the surrounding UI;
        // only the glyphs scale. Reserve exactly the painted width.
        final targetWidth = chip.width + 2 * _chipPaddingX;
        final scale = tailPainter.width == 0
            ? 1.0
            : ((targetWidth - prefixPainter.width) / tailPainter.width).clamp(
                0.01,
                1.0,
              );
        final soloSpacing = tail.isEmpty
            ? (base.letterSpacing ?? 0) + targetWidth - prefixPainter.width
            : base.letterSpacing;
        final tailSpacing =
            tail.isNotEmpty &&
                targetWidth > prefixPainter.width + tailPainter.width
            ? (base.letterSpacing ?? 0) +
                  (targetWidth - prefixPainter.width - tailPainter.width) /
                      tail.length
            : base.letterSpacing;
        prefixPainter.dispose();
        tailPainter.dispose();
        chip.dispose();
        // A chip has one presentation regardless of overlapping bold/code
        // marks; their stored ranges still apply when the block is edited.
        s = base.copyWith(
          color: const Color(0x00000000),
          decoration: TextDecoration.none,
          fontSize: a < prefixEnd
              ? (base.fontSize ?? 16)
              : _dateTailSize(textScaler, base.fontSize ?? 16, scale),
          height: base.height ?? 1.5,
          letterSpacing: tail.isEmpty
              ? soloSpacing
              : (a < prefixEnd ? base.letterSpacing : tailSpacing),
        );
        // Non-breaking spaces keep the chip on a single line, without
        // inserting or removing any UTF-16 code units in the controller.
        segment = segment.replaceAll(' ', '\u00a0');
        semantics = a == mark.start ? label : '';
      }
    }
    spans.add(TextSpan(text: segment, style: s, semanticsLabel: semantics));
  }
  return TextSpan(style: style, children: spans);
}

// A query chip: the icon, the gap after it and the padding (border included)
// around its label.
const _queryIcon = 15.0, _queryGap = 5.0, _queryPadLeft = 7.0, _queryPadRight = 9.0;

/// The label's style: the mockup's 15 px medium weight.
TextStyle queryLabelStyle(TextStyle base) => base.copyWith(
  fontSize: (base.fontSize ?? 16) * 0.94,
  height: 1.4,
  fontWeight: FontWeight.w500,
  decoration: TextDecoration.none,
);

/// The label's runs. A leading count is bold and accented, as in the mockup.
/// A zero count ("no open #work tasks") reads in the tertiary colour.
TextSpan queryLabelSpan(String label, TextStyle style, AppColors c) {
  final zero = label.startsWith('no ');
  final text = zero ? c.textTertiary : c.textPrimary;
  final count = RegExp(r'^(\d+)(.*)$').firstMatch(label);
  if (count == null) {
    return TextSpan(text: label, style: style.copyWith(color: text));
  }
  return TextSpan(
    style: style.copyWith(color: text),
    children: [
      TextSpan(
        text: count.group(1),
        style: style.copyWith(color: c.accentText, fontWeight: FontWeight.w700),
      ),
      TextSpan(text: count.group(2)),
    ],
  );
}

/// The width of a query chip that shows [label] over text styled [base].
double queryChipWidth(String label, TextStyle base, TextScaler scaler) {
  final tp = _layout(
    queryLabelSpan(label, queryLabelStyle(base), AppColors.light),
    scaler,
  );
  final width = _queryPadLeft + _queryIcon + _queryGap + tp.width + _queryPadRight;
  tp.dispose();
  return width;
}

double _textWidth(String text, TextStyle style, TextScaler scaler) {
  final tp = _layout(TextSpan(text: text, style: style), scaler);
  final width = tp.width;
  tp.dispose();
  return width;
}

TextPainter _layout(TextSpan span, TextScaler scaler) => TextPainter(
  text: span,
  textDirection: TextDirection.ltr,
  textScaler: scaler,
)..layout();

/// Split at a complete scalar value while keeping UTF-16 source positions.
int _datePrefixEnd(String text, int start, int end) {
  final first = text.codeUnitAt(start);
  final pair =
      first >= 0xD800 &&
      first <= 0xDBFF &&
      start + 1 < end &&
      text.codeUnitAt(start + 1) >= 0xDC00 &&
      text.codeUnitAt(start + 1) <= 0xDFFF;
  return start + (pair ? 2 : 1);
}

TextStyle _chipTextStyle(TextStyle base, Color color) => base.copyWith(
  fontSize: (base.fontSize ?? 16) * 0.8,
  height: 1.25,
  color: color,
  decoration: TextDecoration.none,
  fontWeight: FontWeight.w500,
);

const _chipPaddingX = 5.0, _chipPaddingY = 1.0;

/// Fit the reserved glyph width using the same scaler as EditableText.
/// Inverting the scaled font size also supports nonlinear accessibility
/// scalers without assuming that scaling a smaller font is proportional.
double _dateTailSize(TextScaler textScaler, double baseSize, double ratio) {
  if (textScaler == TextScaler.noScaling) return baseSize * ratio;
  final target = textScaler.scale(baseSize) * ratio;
  var low = 0.0, high = baseSize;
  for (var i = 0; i < 20; i++) {
    final middle = (low + high) / 2;
    if (textScaler.scale(middle) < target) {
      low = middle;
    } else {
      high = middle;
    }
  }
  return (low + high) / 2;
}

/// Where the character before offset [start] begins.
int mentionGapChar(String t, int start) {
  final u = t.codeUnitAt(start - 1);
  return u >= 0xDC00 && u <= 0xDFFF && start >= 2 ? start - 2 : start - 1;
}

/// The size of a mention icon in text of [fontSize], and the room it takes.
double mentionIconSize(double fontSize) => (fontSize * 1.1).roundToDouble();
double mentionGapFor(double fontSize) => mentionIconSize(fontSize) + 3;

/// Background ranges of [marks]: inline code and background colours.
List<(int, int, Color)> markBackgrounds(List<BMark> marks, AppColors c) => [
  for (final m in marks)
    if (m.kind == BMarkKind.code)
      (m.start, m.end, c.inlineCodeBg)
    else if (m.kind == BMarkKind.color && isBackgroundColor(m.url))
      (m.start, m.end, paletteBackground(m.url, c)!),
];

/// Paints mark backgrounds behind an [EditableText] as rounded rectangles
/// (Inline code has a 4px radius and a little horizontal padding).
class MarkPainter extends CustomPainter {
  MarkPainter({
    required this.controller,
    required this.editable,
    required this.colors,
    required this.textScaler,
    Listenable? icons,
  }) : super(
         repaint: icons == null
             ? controller
             : Listenable.merge([controller, icons]),
       );
  final RichController controller;
  final GlobalKey<EditableTextState> editable;
  final AppColors colors;
  final TextScaler textScaler;

  @override
  void paint(Canvas canvas, Size size) {
    final re = editable.currentState?.renderEditable;
    if (re == null || !re.attached || !re.hasSize) return;
    _paintMentionIcons(canvas, re);
    final bgs = markBackgrounds(controller.marks, colors);
    final len = controller.text.length;
    for (final (s, e, color) in bgs) {
      final a = s.clamp(0, len), b = e.clamp(0, len);
      if (a >= b) continue;
      final paint = Paint()..color = color;
      for (final box in re.getBoxesForSelection(
        TextSelection(baseOffset: a, extentOffset: b),
      )) {
        final r = box.toRect().inflate(0).translate(0, 0);
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTRB(r.left - 2, r.top + 1, r.right + 2, r.bottom - 1),
            const Radius.circular(4),
          ),
          paint,
        );
      }
    }
    _paintDateChips(canvas, re);
    _paintQueryChips(canvas, re);
    _paintPageUnderlines(canvas, re);
  }

  /// Each query chip: a rounded box with its icon and live label, over the
  /// space its covered text was spread to (see [markedSpan]).
  void _paintQueryChips(Canvas canvas, RenderEditable re) {
    final len = controller.text.length;
    for (final mark in controller.marks) {
      if (mark.kind != BMarkKind.query ||
          mark.start < 0 ||
          mark.end > len ||
          mark.start >= mark.end) {
        continue;
      }
      final label = controller.inlineLabelOf(mark);
      if (label == null) continue;
      final boxes = re.getBoxesForSelection(
        TextSelection(baseOffset: mark.start, extentOffset: mark.end),
      );
      if (boxes.isEmpty) continue;
      final box = boxes.first.toRect();
      final base = re.text?.style ?? const TextStyle(fontSize: 16);
      final tp = _layout(
        queryLabelSpan(label, queryLabelStyle(base), colors),
        textScaler,
      );
      final chip = Rect.fromLTWH(
        box.left,
        box.center.dy - (tp.height + 2) / 2,
        queryChipWidth(label, base, textScaler),
        tp.height + 2,
      );
      final rrect = RRect.fromRectAndRadius(chip, const Radius.circular(6));
      canvas.drawRRect(rrect, Paint()..color = colors.bgSecondary);
      canvas.drawRRect(
        rrect,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1
          ..color = colors.border,
      );
      paintIco(
        canvas,
        Ico.search,
        Rect.fromLTWH(
          chip.left + _queryPadLeft,
          chip.center.dy - _queryIcon / 2,
          _queryIcon,
          _queryIcon,
        ),
        colors.textTertiary,
        stroke: 1.3,
      );
      tp.paint(
        canvas,
        Offset(
          chip.left + _queryPadLeft + _queryIcon + _queryGap,
          chip.center.dy - tp.height / 2,
        ),
      );
      tp.dispose();
    }
  }

  void _paintPageUnderlines(Canvas canvas, RenderEditable re) {
    final resolve = controller.pageLinkFill;
    if (resolve == null) return;
    final len = controller.text.length;
    TextPainter? metricPainter;
    for (final mark in controller.marks) {
      final target = inlinePageTarget(mark);
      if (target == null ||
          mark.start < 0 ||
          mark.end > len ||
          mark.start >= mark.end) {
        continue;
      }
      final fill = resolve(target);
      if (fill == null) continue;
      metricPainter ??= TextPainter(
        text: re.text,
        textDirection: re.textDirection,
        textScaler: textScaler,
      )..layout(maxWidth: re.size.width);
      final metrics = metricPainter.computeLineMetrics();
      final boxes = re.getBoxesForSelection(
        TextSelection(baseOffset: mark.start, extentOffset: mark.end),
      );
      for (final box in boxes) {
        final rect = box.toRect();
        // Tight selection boxes share the line's descent; account for scroll
        // by using their local bottom rather than an absolute line position.
        final line = metrics.reduce(
          (a, b) =>
              (a.baseline + a.descent - rect.bottom).abs() <
                  (b.baseline + b.descent - rect.bottom).abs()
              ? a
              : b,
        );
        canvas.drawRect(
          Rect.fromLTWH(
            rect.left,
            rect.bottom - line.descent + 2,
            rect.width,
            3,
          ),
          Paint()..color = fill,
        );
      }
    }
    metricPainter?.dispose();
  }

  void _paintDateChips(Canvas canvas, RenderEditable re) {
    if (!controller.dateChips) return;
    final len = controller.text.length;
    for (final mark in controller.marks) {
      if (mark.kind != BMarkKind.date ||
          mark.start < 0 ||
          mark.end > len ||
          mark.start >= mark.end) {
        continue;
      }
      final day = todoDate(mark.url);
      if (day == null) continue;
      final boxes = re.getBoxesForSelection(
        TextSelection(baseOffset: mark.start, extentOffset: mark.end),
      );
      if (boxes.isEmpty) continue;
      final box = boxes.first.toRect();
      final overdue = !controller.todoChecked && todoDateDistance(day) < 0;
      final tag = colors.tag(overdue ? 'red' : 'gray');
      final base = re.text?.style ?? const TextStyle(fontSize: 16);
      final tp = TextPainter(
        text: TextSpan(
          text: todoDateLabel(day),
          style: _chipTextStyle(
            base,
            overdue ? tag.text : colors.textSecondary,
          ),
        ),
        textDirection: TextDirection.ltr,
        textScaler: textScaler,
      )..layout();
      final chip = Rect.fromLTWH(
        box.left,
        box.center.dy - (tp.height + 2 * _chipPaddingY) / 2,
        tp.width + 2 * _chipPaddingX,
        tp.height + 2 * _chipPaddingY,
      );
      canvas.drawRRect(
        RRect.fromRectAndRadius(chip, const Radius.circular(4)),
        Paint()..color = tag.bg,
      );
      tp.paint(
        canvas,
        Offset(chip.left + _chipPaddingX, chip.center.dy - tp.height / 2),
      );
      tp.dispose();
    }
  }

  /// The icon of each page mention, in the room left in front of it.
  void _paintMentionIcons(Canvas canvas, RenderEditable re) {
    final gap = controller.mentionGap;
    if (gap <= 0) return;
    final len = controller.text.length;
    for (final m in controller.marks) {
      if (m.kind != BMarkKind.pageLink ||
          m.start < 0 ||
          m.start >= m.end ||
          m.end > len) {
        continue;
      }
      final boxes = re.getBoxesForSelection(
        TextSelection(baseOffset: m.start, extentOffset: m.start + 1),
      );
      if (boxes.isEmpty) continue;
      final b = boxes.first.toRect();
      final icon = gap - 3;
      final r = Rect.fromLTWH(
        b.left - gap + 1,
        b.center.dy - icon / 2,
        icon,
        icon,
      );
      final emoji = controller.mentionIcon?.call(m.url);
      if (emoji != null && emoji.isNotEmpty) {
        final tp = TextPainter(
          text: TextSpan(
            text: emoji,
            style: TextStyle(
              fontSize: icon * 0.85,
              height: 1,
              fontFamilyFallback: emojiFallback,
            ),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        tp.paint(canvas, r.center - Offset(tp.width / 2, tp.height / 2));
        tp.dispose();
      } else {
        paintIco(canvas, Ico.page, r, colors.iconSecondary, stroke: 1.3);
      }
    }
  }

  @override
  bool shouldRepaint(MarkPainter old) =>
      old.controller != controller ||
      old.colors != colors ||
      old.textScaler != textScaler;
}

/// Authored page mentions and ordinary links using the app's page URL.
String? inlinePageTarget(BMark mark) {
  if (mark.kind == BMarkKind.pageLink) return mark.url;
  if (mark.kind != BMarkKind.link) return null;
  return PageLink.parse(mark.url)?.pageId;
}
