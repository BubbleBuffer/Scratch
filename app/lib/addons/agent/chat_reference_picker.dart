import 'outline_names.dart';
// The Chat picker for a chat_ref block: chats, then a chat's chapters, then a
// chapter's sections (a chapter with no sections is picked directly). It only
// returns the reference; the block's insert body fills the block from it.

import 'package:flutter/widgets.dart';

import '../../src/rust/api/agent.dart' show OutlineChapter;
import '../../ui/ui.dart';
import 'chat_reference.dart';

/// A chat and its outline, as the picker lists them.
class ChatRefChat {
  const ChatRefChat({
    required this.sessionId,
    required this.title,
    required this.chapters,
  });
  final String sessionId;
  final String title;
  final List<OutlineChapter> chapters;
}

/// Shows the picker. Completes with the reference to insert, or null.
Future<ChatReference?> showChatReferencePicker(
  BuildContext context, {
  required Future<List<ChatRefChat>> Function() load,
}) => showPickerSheet<ChatReference>(
  context,
  (_) => ChatReferencePicker(load: load),
  desktopWidth: 480,
  shrink: true,
);

/// The picker's content. Public for tests.
class ChatReferencePicker extends StatefulWidget {
  const ChatReferencePicker({super.key, required this.load});
  final Future<List<ChatRefChat>> Function() load;

  @override
  State<ChatReferencePicker> createState() => _ChatReferencePickerState();
}

class _ChatReferencePickerState extends State<ChatReferencePicker> {
  late final Future<List<ChatRefChat>> _chats = widget.load();
  ChatRefChat? _chat;
  OutlineChapter? _chapter;

  void _pick(ChatReference r) => Navigator.of(context).pop(r);

  void _back() => setState(() {
    if (_chapter != null) {
      _chapter = null;
    } else {
      _chat = null;
    }
  });

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 480),
      child: PickerSheet(
        header: Padding(
          padding: const EdgeInsets.fromLTRB(
            Layout.gutter,
            14,
            Layout.gutter,
            8,
          ),
          child: Text('Chat reference', style: tsDisplay(ty.h3, c.textPrimary)),
        ),
        listHeight: 360,
        list: FutureBuilder<List<ChatRefChat>>(
          future: _chats,
          builder: (context, snap) {
            if (snap.hasError) {
              return _Empty('Chats are unavailable right now', ty: ty, c: c);
            }
            if (!snap.hasData) {
              return _Empty('Loading chats…', ty: ty, c: c);
            }
            final chats = snap.data!;
            if (chats.isEmpty) {
              return _Empty('No chats yet', ty: ty, c: c);
            }
            final chat = _chat;
            if (chat == null) {
              return Column(
                children: [
                  for (final ch in chats)
                    SheetItem(
                      key: ValueKey('picker-chat-${ch.sessionId}'),
                      label: ch.title,
                      icon: Ico.comment,
                      onTap: () => setState(() => _chat = ch),
                    ),
                ],
              );
            }
            final chapter = _chapter;
            if (chapter == null) {
              return Column(
                children: [
                  _BackRow(label: chat.title, onTap: _back),
                  if (chat.chapters.isEmpty)
                    _Empty('No chapters yet', ty: ty, c: c),
                  for (final chp in chat.chapters)
                    SheetItem(
                      key: ValueKey('picker-chapter-${chp.id}'),
                      label: outlineNameGist(chp.title, chp.summary).name,
                      icon: Ico.page,
                      onTap: () {
                        if (chp.sections.isEmpty) {
                          _pick(
                            ChatReference.fromChapter(
                              sessionId: chat.sessionId,
                              chatTitle: chat.title,
                              chapter: chp,
                            ),
                          );
                        } else {
                          setState(() => _chapter = chp);
                        }
                      },
                    ),
                ],
              );
            }
            return Column(
              children: [
                _BackRow(
                  label: outlineNameGist(chapter.title, chapter.summary).name,
                  onTap: _back,
                ),
                SheetItem(
                  key: const ValueKey('picker-whole-chapter'),
                  label: 'Whole chapter',
                  icon: Ico.page,
                  onTap: () => _pick(
                    ChatReference.fromChapter(
                      sessionId: chat.sessionId,
                      chatTitle: chat.title,
                      chapter: chapter,
                    ),
                  ),
                ),
                for (final s in chapter.sections)
                  SheetItem(
                    key: ValueKey('picker-section-${s.id}'),
                    label: outlineNameGist(
                      s.title,
                      s.summary,
                      chapterName: outlineNameGist(
                        chapter.title,
                        chapter.summary,
                      ).name,
                    ).name,
                    icon: Ico.text,
                    onTap: () => _pick(
                      ChatReference.fromSection(
                        sessionId: chat.sessionId,
                        chatTitle: chat.title,
                        chapter: chapter,
                        section: s,
                      ),
                    ),
                  ),
              ],
            );
          },
        ),
      ),
    );
  }
}

class _BackRow extends StatelessWidget {
  const _BackRow({required this.label, required this.onTap});
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) =>
      SheetItem(label: label, icon: Ico.chevronLeft, onTap: onTap);
}

class _Empty extends StatelessWidget {
  const _Empty(this.text, {required this.ty, required this.c});
  final String text;
  final TypeScale ty;
  final AppColors c;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.all(16),
    child: Text(text, style: ts(ty.ui, c.textTertiary)),
  );
}
