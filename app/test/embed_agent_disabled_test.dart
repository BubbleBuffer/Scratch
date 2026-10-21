import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/embed_block.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import 'package:notes_app/shell/addons.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;

import 'harness.dart';

void main() {
  setUpAll(() async {
    registerBuiltins(disabledAddons: {'agent'});
    await initCore();
  });

  appTest('a stored generic chat embed opens with Agent disabled at boot', (
    tester,
  ) async {
    final f = await pumpApp(tester, mobile: false, empty: true);
    const target = 'chat:offline-session';
    final host = await core(tester, (v) => v.createPage(title: 'Chat host'));
    await core(
      tester,
      (v) => v.run(
        (native) => native.upsertSearchRow(
          source: 'agent',
          row: const ExternalSearchRow(
            id: target,
            title: 'Saved chat title',
            body: 'Saved chapter summary',
            pageType: 'chat',
            target: '{"sessionId":"offline-session","level":"chat"}',
            updatedMs: 1,
          ),
        ),
        writes: true,
      ),
    );
    await core(
      tester,
      (v) => v.apply(host, [
        const BOp(
          op: OpKind.insert,
          id: 'e',
          kind: embedKind,
          text: '',
          marks: [],
          checked: false,
        ),
        const BOp(
          op: OpKind.setTarget,
          id: 'e',
          kind: embedKind,
          target: target,
          text: '',
          marks: [],
          checked: false,
        ),
      ]),
    );
    nav.openPage(tester.element(find.byType(HomeScreen)), host, 'Chat host');
    await settle(tester);
    expect(noteTypeRegistry.of('chat'), isNull);
    expect(AgentStore.of(f.app).enabled, isFalse);
    expect(find.text('Saved chat title'), findsOneWidget);
    expect(find.text('Unsupported note type (chat)'), findsOneWidget);
    expect(find.text('Saved chapter summary'), findsOneWidget);
    final block = (await core(
      tester,
      (v) => v.loadPage(host),
    )).blocks.singleWhere((b) => b.id == 'e');
    expect(block.kind, embedKind);
    expect(block.target, target);
    expect(block.text, isEmpty);
  });
}
