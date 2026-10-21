import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';

import 'harness.dart';

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    for (final toggle in [false, true]) {
      appTest(
        'empty and long progress heading remains editable mobile=$mobile toggle=$toggle',
        (tester) async {
          final fixture = await pumpApp(tester, mobile: mobile);
          if (mobile) tester.view.physicalSize = const Size(320 * 2, 812 * 2);
          final kind = toggle ? Kinds.toggleHeading2 : Kinds.heading2;
          await core(tester, (vault) async {
            await vault.apply(fixture.showcaseId, [
              BOp(
                op: OpKind.setKind,
                id: 'sc02',
                kind: kind,
                text: '',
                marks: const [],
                checked: false,
              ),
              BOp(
                op: OpKind.setText,
                id: 'sc02',
                kind: kind,
                text: '',
                marks: const [],
                checked: false,
              ),
              BOp(
                op: OpKind.insert,
                id: 'progress-one',
                after: 'sc02',
                kind: Kinds.todo,
                text: 'First task',
                marks: const [],
                checked: true,
              ),
              BOp(
                op: OpKind.insert,
                id: 'progress-two',
                after: 'progress-one',
                kind: Kinds.todo,
                text: 'Second task',
                marks: const [],
                checked: false,
              ),
            ]);
            await vault.flush();
          });
          await settle(tester);
          await openShowcase(tester);
          final empty = editableWith('');
          await tester.ensureVisible(empty);
          await settle(tester);
          expect(tester.getSize(empty).width, greaterThan(40));
          expect(
            find.byKey(const ValueKey('heading-progress-sc02')),
            findsOneWidget,
          );
          final placeholder = toggle ? 'Toggle heading 2' : 'Heading 2';
          final label = find.text(placeholder).first;
          expect(tester.getSize(label).width, greaterThan(40));
          await tapAndSettle(tester, empty);
          expect(editState(tester, empty).widget.focusNode.hasFocus, isTrue);
          const long =
              'A very long heading that still wraps while its task progress remains beside it on a narrow phone screen';
          await tester.enterText(empty, long);
          await settle(tester);
          final edited = editableWith(long);
          expect(edited, findsOneWidget);
          if (mobile) expect(tester.getSize(edited).height, greaterThan(50));
          final count = find.byKey(const ValueKey('heading-progress-sc02'));
          expect(
            tester.getTopRight(count).dx,
            lessThanOrEqualTo(mobile ? 320 : 1440),
          );
          expect(
            (await blockById(tester, fixture.showcaseId, 'sc02')).text,
            long,
          );
          expect(tester.takeException(), isNull);
        },
      );
    }
  }
}
