@Tags(['golden'])
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';

import 'db_harness.dart';

void main() {
  setUpAll(initCore);
  for (final dark in [false, true]) {
    dbTest('Stacks row properties, callout and section progress dark=$dark', (
      tester,
    ) async {
      await pumpDb(tester, mobile: false, dark: dark);
      await core(tester, (vault) async {
        final rows = await vault.run(
          (v) => v.query(
            scope: db.projects,
            filter: null,
            sorts: const [],
            offset: 0,
            limit: 20,
          ),
        );
        final row = rows.rows.firstWhere((p) => p.title == 'Mobile app launch');
        await vault.apply(row.id, [
          BOp(
            op: OpKind.insert,
            id: 'stacks-note',
            kind: Kinds.callout,
            text: 'Keep the launch checklist together. Tick each item as it lands.',
            marks: const [],
            checked: false,
          ),
          BOp(
            op: OpKind.insert,
            id: 'stacks-heading',
            after: 'stacks-note',
            kind: Kinds.heading2,
            text: 'Launch checklist',
            marks: const [],
            checked: false,
          ),
          BOp(
            op: OpKind.insert,
            id: 'stacks-one',
            after: 'stacks-heading',
            kind: Kinds.todo,
            text: 'Review the store listing',
            marks: const [],
            checked: true,
          ),
          BOp(
            op: OpKind.insert,
            id: 'stacks-two',
            after: 'stacks-one',
            kind: Kinds.todo,
            text: 'Finish the release notes',
            marks: const [],
            checked: false,
          ),
          BOp(
            op: OpKind.insert,
            id: 'stacks-three',
            after: 'stacks-two',
            kind: Kinds.todo,
            text: 'Check the launch build',
            marks: const [],
            checked: false,
          ),
        ]);
        await vault.flush();
      });
      await settle(tester);
      await openView(tester, 'Board', mobile: false);
      await tapKey(tester, 'card-Mobile app launch');
      await shot('stacks-desktop-${dark ? 'dark' : 'light'}-page-blocks');
    });
  }
}
