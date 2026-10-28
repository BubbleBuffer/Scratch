// Temporary probe (removed before merge): where the rich page's blocks sit
// on a phone before the golden test taps toggle-r06.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'harness.dart';
import 'pages_fixture.dart';

void main() {
  setUpAll(initCore);
  for (final mobile in [true, false]) {
    appTest('probe rich page mobile=$mobile', (tester) async {
      final tv = await pumpApp(tester, mobile: mobile, dark: false);
      await createRichPage(tester, tv);
      await openByTitle(tester, richTitle);
      final view = tester.getSize(find.byType(Directionality).first);
      // ignore: avoid_print
      print('PROBE mobile=$mobile view=$view');
      final seen = <String>{};
      for (final e in find.byWidgetPredicate((w) {
        final k = w.key;
        return k is ValueKey<String> &&
            RegExp(r'(^|-)r[0-2]\d($|-)').hasMatch(k.value);
      }).evaluate()) {
        final k = (e.widget.key! as ValueKey<String>).value;
        if (!seen.add(k)) continue;
        final box = e.renderObject;
        final rect = box is RenderBox && box.hasSize
            ? box.localToGlobal(Offset.zero) & box.size
            : null;
        // ignore: avoid_print
        print('PROBE key=$k rect=$rect');
      }
      for (final s in find.byType(Scrollable).evaluate()) {
        final p = (s as StatefulElement).state as ScrollableState;
        final pos = p.position;
        if (!pos.hasContentDimensions) continue;
        // ignore: avoid_print
        print(
          'PROBE scrollable axis=${pos.axis} pixels=${pos.pixels} '
          'max=${pos.maxScrollExtent} viewport=${pos.viewportDimension}',
        );
      }
      // ignore: avoid_print
      print(
        'PROBE toggle-r06 found=${find.byKey(const ValueKey('toggle-r06')).evaluate().length}',
      );
    });
  }
}
