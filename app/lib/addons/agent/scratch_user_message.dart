// The user's message as the chat draws it: the text under chips for the pages,
// chats and editor selection it was sent with. Scratch resolves the chips: a
// page chip takes its title from the vault and opens the page, a chat chip
// opens the chat.

import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/ui.dart';
import 'agent_transcript.dart';
import 'navigation.dart';

class ScratchUserMessage extends StatelessWidget {
  const ScratchUserMessage({
    super.key,
    required this.message,
    required this.onOpenChat,
    required this.chatTitle,
    this.sessionId,
  });
  final UserText message;
  final String? sessionId;
  final ValueChanged<String> onOpenChat;
  final String Function(String id, String raw) chatTitle;
  Widget _chip(
    BuildContext context,
    String label,
    Key key,
    Ico icon,
    VoidCallback onTap, {
    required double availableWidth,
  }) {
    final c = context.colors;
    return Hover(
      key: key,
      onTap: onTap,
      radius: 14,
      border: Border.all(color: c.borderSubtle),
      background: c.bgPrimary,
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon20(icon, size: 12, color: c.iconTertiary),
          const SizedBox(width: 5),
          ConstrainedBox(
            constraints: BoxConstraints(
              maxWidth: (availableWidth - 35).clamp(0.0, 200.0).toDouble(),
            ),
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(context.type.uiSmall, c.textSecondary),
            ),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final selection = message.context?.selection;
    final pages = context.watch<PagesStore>();
    return Align(
      alignment: Alignment.centerRight,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 560),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 9),
          decoration: BoxDecoration(
            color: c.bgSecondary,
            borderRadius: Radii.panel,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              if ((message.context?.pages.isNotEmpty ?? false) ||
                  (message.context?.chats.isNotEmpty ?? false) ||
                  selection != null) ...[
                LayoutBuilder(
                  builder: (context, bounds) => Wrap(
                    spacing: 4,
                    runSpacing: 4,
                    children: [
                      for (final page in message.context?.pages ?? [])
                        _chip(
                          context,
                          displayTitle(
                            page.title,
                            firstLine: pages.meta(page.id)?.firstLine ?? '',
                          ),
                          ValueKey('agent-msg-${message.id}-page-${page.id}'),
                          Ico.page,
                          () => openAgentPage(
                            context,
                            page.id,
                            page.title,
                            sessionId: sessionId,
                          ),
                          availableWidth: bounds.maxWidth,
                        ),
                      for (final chat in message.context?.chats ?? [])
                        _chip(
                          context,
                          chatTitle(chat.id, chat.title),
                          ValueKey('agent-msg-${message.id}-chat-${chat.id}'),
                          Ico.comment,
                          () => onOpenChat(chat.id),
                          availableWidth: bounds.maxWidth,
                        ),
                      if (selection != null)
                        _chip(
                          context,
                          'Selection',
                          ValueKey('agent-msg-${message.id}-selection'),
                          Ico.text,
                          () => openAgentPage(
                            context,
                            selection.pageId,
                            selection.pageTitle,
                            blocks: selection.blockIds,
                            sessionId: sessionId,
                          ),
                          availableWidth: bounds.maxWidth,
                        ),
                    ],
                  ),
                ),
                const SizedBox(height: 6),
              ],
              Text(message.text, style: ts(context.type.body, c.textPrimary)),
            ],
          ),
        ),
      ),
    );
  }
}
