import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/shell/search.dart';
import 'package:notes_app/src/rust/api/types.dart';

import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);

  appTest('home page search retains pages when chats exceed the result limit', (
    tester,
  ) async {
    final fixture = await pumpApp(tester, mobile: false, empty: true);
    final vault = fixture.app.vault;
    final page = await real(tester, () async {
      final id = await vault.createPage(title: 'Budget plan');
      for (var i = 0; i < 120; i++) {
        await vault.run(
          (v) => v.upsertSearchRow(
            source: 'agent',
            row: ExternalSearchRow(
              id: 'chat-$i',
              title: 'Budget',
              body: '',
              pageType: 'chat',
              target: null,
              updatedMs: 0,
            ),
          ),
        );
      }
      await vault.flush();
      return id;
    });
    final search = PageSearch(vault);
    addTearDown(search.dispose);
    await real(tester, () => search.run('Budget'));
    expect(search.hits!.map((hit) => hit.id), [page]);
    expect(search.hits!.every((hit) => hit.source == 'vault'), isTrue);
  });
}
