import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/rich_controller.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart' show loadFonts;

void main() {
  for (final mobile in [false, true]) {
    for (final scale in [1.0, 2.0]) {
      testWidgets('date chip keeps line metrics and UTF-16 caret offsets on ${mobile ? 'mobile' : 'desktop'} at $scale scaling', (tester) async {
        final textScaler = TextScaler.linear(scale);
        Clock.fixedAt = DateTime(2026, 10, 6, 11, 20);
        addTearDown(() => Clock.fixedAt = null);
        await tester.runAsync(loadFonts);
        sansFamily = 'Atkinson Hyperlegible Next';
        monoFamily = 'Fragment Mono';
        final base = ts(mobile ? TypeScale.mobile.body : TypeScale.desktop.body, AppColors.light.textPrimary);
        late BuildContext editableContext;
        await tester.pumpWidget(MediaQuery(
          data: MediaQueryData(textScaler: textScaler),
          child: Builder(builder: (context) {
            editableContext = context;
            return const SizedBox.shrink();
          }),
        ));
        const text = '🌱 Task @October 8, 2026 next';
        final start = text.indexOf('@'), end = text.indexOf(' next');
        final date = BMark(start: start, end: end, kind: BMarkKind.date, url: '2026-10-08');
        final plain = TextPainter(text: TextSpan(text: text, style: base), textDirection: TextDirection.ltr, textScaler: textScaler)..layout();
        final lineHeight = plain.computeLineMetrics().single.height;
        plain.dispose();
        final label = TextPainter(text: TextSpan(text: 'Thursday', style: base.copyWith(
            fontSize: base.fontSize! * 0.8, height: 1.25, fontWeight: FontWeight.w500)), textDirection: TextDirection.ltr, textScaler: textScaler)..layout();
        final targetWidth = label.width + 10;
        label.dispose();
        final cases = [
          [date],
          [date, BMark(start: start, end: end, kind: BMarkKind.bold, url: '')],
          [date, BMark(start: start + 2, end: end - 2, kind: BMarkKind.bold, url: '')],
          [date, BMark(start: start, end: end, kind: BMarkKind.code, url: '')],
        ];
        for (final marks in cases) {
          final controller = RichController(text: text, marks: marks)..dateChips = true;
          final span = controller.buildTextSpan(context: editableContext, style: base, withComposing: false);
          controller.dispose();
          expect(span.toPlainText(includeSemanticsLabels: false).length, text.length);
          final tp = TextPainter(text: span, textDirection: TextDirection.ltr, textScaler: textScaler)..layout(maxWidth: 700);
          expect(tp.computeLineMetrics().single.height, lineHeight);
          final boxes = tp.getBoxesForSelection(TextSelection(baseOffset: start, extentOffset: end));
          final width = boxes.fold<double>(0, (sum, b) => sum + b.right - b.left);
          expect(width, closeTo(targetWidth, 0.1));
          for (final offset in [0, start, end, text.length]) {
            final caret = tp.getOffsetForCaret(TextPosition(offset: offset), Rect.zero);
            expect(tp.getPositionForOffset(caret + const Offset(0.1, 1)).offset, offset);
          }
          tp.layout(maxWidth: 95 * scale);
          final wrapped = tp.getBoxesForSelection(TextSelection(baseOffset: start, extentOffset: end));
          final lines = tp.computeLineMetrics();
          expect(lines.every((line) => line.height == lineHeight), isTrue);
          // Every piece of this transparent mention belongs to one line.
          final occupied = wrapped.map((b) => lines.indexWhere((line) =>
              b.top >= line.baseline - line.ascent && b.bottom <= line.baseline + line.descent + 0.1)).toSet();
          expect(occupied, isNot(contains(-1)));
          expect(occupied.length, 1);
          tp.dispose();
          final literal = text.substring(start, end);
          final soloMarks = marks.map((m) => BMark(start: m.start - start, end: m.end - start, kind: m.kind, url: m.url)).toList();
          final solo = TextPainter(text: markedSpan(literal, soloMarks, AppColors.light, base,
              dateChips: true, textScaler: textScaler), textDirection: TextDirection.ltr, textScaler: textScaler)..layout();
          expect(solo.computeLineMetrics().single.height, lineHeight);
          solo.dispose();
        }
      });
    }
  }
}
