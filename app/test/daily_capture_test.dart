import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/daily_capture.dart';
import 'package:notes_app/state/agenda_store.dart';
import 'package:flutter_test/flutter_test.dart';

import 'harness.dart';

void main() {
  setUpAll(initCore);

  appTest(
    'quick capture inserts tasks and writing above Log without changing it',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final page = await core(tester, (vault) => todaysNote(vault));
      final existing = await pageOf(tester, page);
      final rootLog = existing.blocks
          .where(
            (b) => b.depth == 0 && b.kind == Kinds.toggle && b.text == 'Log',
          )
          .firstOrNull;
      final log = rootLog?.id ?? newId();
      final line = newId();
      await core(
        tester,
        (vault) => vault.apply(page, [
          if (rootLog == null)
            BOp(
              op: OpKind.insert,
              id: log,
              after: existing.blocks.where((b) => b.depth == 0).lastOrNull?.id,
              kind: Kinds.toggle,
              text: 'Log',
              checked: false,
              marks: const [],
            ),
          BOp(
            op: OpKind.insert,
            id: line,
            parent: log,
            kind: Kinds.paragraph,
            text: '11:20  Existing log entry',
            checked: false,
            marks: const [],
          ),
        ]),
      );
      expect(
        await core(
          tester,
          (vault) => captureToday(vault, 'Captured task', task: true),
        ),
        page,
      );
      expect(
        await core(
          tester,
          (vault) => captureToday(vault, 'Captured writing', task: false),
        ),
        page,
      );
      final saved = await pageOf(tester, page);
      final roots = saved.blocks.where((b) => b.depth == 0).toList();
      expect(roots.last.id, log);
      expect(roots[roots.length - 2].text, 'Captured writing');
      expect(roots[roots.length - 3].text, startsWith('Captured task @'));
      expect(roots[roots.length - 3].marks.single.url, '2026-10-06');
      expect(
        saved.blocks.singleWhere((b) => b.id == line).text,
        '11:20  Existing log entry',
      );
      expect(
        (await core(
          tester,
          (vault) => vault.run(
            (v) => v.dailyNote(
              dayMs: dayStamp(fixedNow),
              title: '',
              create: false,
            ),
          ),
        )),
        page,
      );
    },
  );
}
