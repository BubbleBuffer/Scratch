// Unknown block kinds render the "Not supported on this version" card. The
// card is read-only: no text field, and the kind and text stay as stored when
// a neighbouring paragraph is edited.
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';

import 'harness.dart';
import 'sync_helpers.dart' show real;

const queryKind = 'future_kind';
const queryText = 'tag:work';
const blankKind = 'mod:blank kind';
const neighbour = 'Neighbour paragraph';
const wording = 'Not supported on this version of Scratch';

Future<({String page, String paragraph, String query, String blank})> _seed(
  WidgetTester tester,
  TestVault tv,
) async {
  final page = tv.pages['Garden plan']!;
  final paragraph = newId(), query = newId(), blank = newId();
  await real(
    tester,
    () => tv.vault.apply(page, [
      BOp(
        op: OpKind.insert,
        id: paragraph,
        kind: 'paragraph',
        text: neighbour,
        marks: const [],
        checked: false,
      ),
      BOp(
        op: OpKind.insert,
        id: query,
        kind: queryKind,
        text: queryText,
        marks: const [],
        checked: false,
      ),
      BOp(
        op: OpKind.insert,
        id: blank,
        kind: blankKind,
        text: '',
        marks: const [],
        checked: false,
      ),
    ]),
  );
  await real(tester, () => tv.vault.flush());
  await tapAndSettle(tester, find.text('Garden plan').last);
  await settle(tester);
  return (page: page, paragraph: paragraph, query: query, blank: blank);
}

void main() {
  setUpAll(initCore);

  appTest('unknown kinds show the card with kind and text, and no text field', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    await _seed(tester, tv);

    expect(find.text(wording), findsNWidgets(2));
    expect(find.text('$queryKind · $queryText'), findsOneWidget);
    expect(find.text(blankKind), findsOneWidget);
    expect(editableWith(queryText), findsNothing);
    expect(tester.takeException(), isNull);
  });

  appTest('the card renders in dark without exceptions', (tester) async {
    final tv = await pumpApp(tester, mobile: false, dark: true);
    await _seed(tester, tv);
    expect(find.text(wording), findsNWidgets(2));
    expect(tester.takeException(), isNull);
  });

  appTest('typing beside an unknown block leaves its kind and text unchanged', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final s = await _seed(tester, tv);

    await focusAt(tester, editableWith(neighbour), neighbour.length);
    await typeText(tester, ' edited');
    await settle(tester, min: const Duration(seconds: 1));
    await real(tester, () => tv.vault.flush());

    final blocks = await blocksOf(tester, s.page);
    final paragraph = blocks.singleWhere((b) => b.id == s.paragraph);
    expect(paragraph.text, '$neighbour edited');
    final query = blocks.singleWhere((b) => b.id == s.query);
    expect(query.kind, queryKind);
    expect(query.text, queryText);
    final blank = blocks.singleWhere((b) => b.id == s.blank);
    expect(blank.kind, blankKind);
    expect(blank.text, '');
    expect(tester.takeException(), isNull);
  });
}
