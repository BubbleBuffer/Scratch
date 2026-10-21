// A chat_ref block through the native bridge: inserted with the cached
// reference as its target and its summary as its text, the vault is closed
// and opened again, and the block loads back with its kind, text and decoded
// payload unchanged. Runs on a vault in a temporary folder, like sync tests.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/addons/agent/api.dart';

import 'chat_reference_fixture.dart';
import 'harness.dart';

void main() {
  setUpAll(initCore);

  test(
    'a chat_ref block keeps its kind, summary and payload after a reopen',
    () async {
      final dir = Directory.systemTemp.createTempSync('notes_chatref_');
      final path = '${dir.path}${Platform.pathSeparator}vault';
      final ref = ChatReference.fromChapter(
        sessionId: 'sess-1',
        chatTitle: chatRefChatTitle,
        chapter: chatRefChapter(),
      );
      final blockId = newId();
      Vault? opened;
      try {
        var vault = await Vault.open(path);
        opened = vault;
        final pageId = await vault.createPage(title: 'Chat ref round trip');
        await vault.apply(pageId, [
          BOp(
            op: OpKind.insert,
            id: blockId,
            after: null,
            kind: Kinds.chatRef,
            text: ref.searchText,
            marks: const [],
            checked: false,
          ),
          BOp(
            op: OpKind.setTarget,
            id: blockId,
            kind: Kinds.chatRef,
            text: '',
            marks: const [],
            checked: false,
            target: ref.toTarget(),
          ),
        ]);
        await vault.flush();
        vault.dispose();
        opened = null;

        vault = await Vault.open(path);
        opened = vault;
        final page = await vault.loadPage(pageId);
        final block = page.blocks.singleWhere((b) => b.id == blockId);

        expect(block.kind, Kinds.chatRef);
        expect(block.kind, 'chat_ref');
        expect(
          block.text,
          ref.searchText,
          reason: 'the summary is the searchable text',
        );
        expect(block.target, isNotNull);
        expect(
          jsonDecode(block.target!),
          jsonDecode(ref.toTarget()),
          reason: 'the whole payload decodes unchanged',
        );

        final back = ChatReference.fromTarget(block.target);
        expect(
          back,
          isNotNull,
          reason: 'the stored target reads back as a reference',
        );
        expect(back!.toJson(), ref.toJson());
        expect(back.label, ref.label);
        expect(back.sections.map((s) => s.firstMessageId), ['m-40', 'm-53']);
      } finally {
        opened?.dispose();
        try {
          await dir.delete(recursive: true);
        } catch (_) {}
      }
    },
  );
}
