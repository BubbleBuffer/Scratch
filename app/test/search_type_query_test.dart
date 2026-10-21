import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/ui/database/values.dart';
import 'package:notes_app/shell/search.dart';

import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);

  appTest('type chips preserve an actual property named type', (tester) async {
    final fixture = await pumpApp(tester, mobile: false, empty: true);
    final vault = fixture.app.vault;
    await real(tester, () async {
      final page = await vault.createPage(title: 'Typed property');
      await vault.run(
        (v) => v.setProperty(
          page: page,
          key: 'type',
          value: Values.text('document'),
        ),
      );
    });
    const query = 'type:chat "type":document';
    final parsed = await real(
      tester,
      () => vault.run((v) => v.parseSearch(q: query)),
    );
    expect(parsed.spec.operators.last.property, isTrue);
    final search = PageSearch(vault, includeExternal: true);
    addTearDown(search.dispose);
    await real(tester, () => search.run(query));
    expect(search.activeTypes, {'chat'});
    final changed = await real(
      tester,
      () => search.rewriteTypes(query, {'database'}),
    );
    expect(changed, '"type":document type:database');
  });
}
