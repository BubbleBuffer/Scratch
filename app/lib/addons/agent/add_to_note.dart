import '../../services/addon_services.dart';
// "Add to note" for a chat reference: choose how it shows (card, or link in
// text), then the note it goes into (recent notes, search, or a new note).
// The reference is appended as the last top-level block of that note, with
// a notice that opens the note. Uses the real vault and PagesStore.

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'chat_reference.dart';
import 'chat_reference_widgets.dart';

/// Shows the "Add to note" sheet (a bottom sheet on a phone, a centred panel
/// elsewhere) for [reference].
Future<void> showAddChatReference(
  BuildContext context,
  ChatReference reference,
) => showPickerSheet<void>(
  context,
  (_) => AddChatReferenceBox(reference: reference),
  desktopWidth: 520,
  desktopScroll: true,
);

/// The sheet's content. Public for tests.
class AddChatReferenceBox extends StatefulWidget {
  const AddChatReferenceBox({super.key, required this.reference});
  final ChatReference reference;

  @override
  State<AddChatReferenceBox> createState() => _AddChatReferenceBoxState();
}

/// The target when the user picks "New note".
const _newNoteTarget = '\u0000new';

/// A note offered as a target: from recent pages or a search hit.
class _NoteRow {
  const _NoteRow(this.id, this.title, this.icon);
  final String id;
  final String title;
  final String? icon;
}

/// The label a note row shows: its title, then its first body line, then
/// "Untitled" (see [displayTitle]).
String _noteLabel(PagesStore pages, _NoteRow n) =>
    displayTitle(n.title, firstLine: pages.meta(n.id)?.firstLine ?? '');

class _AddChatReferenceBoxState extends State<AddChatReferenceBox> {
  // Link in text is shown but cannot be added yet: a chat range needs a click
  // handler in the page editor before a link can open it.
  static const _linkInTextReady = false;

  final _query = TextEditingController();
  final _focus = FocusNode();
  bool _asLink = false;
  bool _busy = false;
  int _searchGen = 0;

  late List<_NoteRow> _notes = _recent();
  late String? _target = _notes.isEmpty ? _newNoteTarget : _notes.first.id;

  List<_NoteRow> _recent() => [
    for (final r in context.app.pages.first(6)) _NoteRow(r.id, r.title, r.icon),
  ];

  @override
  void dispose() {
    _query.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _search(String text) async {
    final q = text.trim();
    final gen = ++_searchGen;
    if (q.isEmpty) {
      setState(() => _notes = _recent());
      return;
    }
    final hits = await context.app.vault.search(q, limit: 30);
    if (!mounted || gen != _searchGen) return;
    setState(() {
      _notes = [for (final h in hits) _NoteRow(h.id, h.title, null)];
      if (_target != _newNoteTarget && !_notes.any((n) => n.id == _target)) {
        _target = null;
      }
    });
  }

  bool get _canAdd =>
      !_busy && (!_asLink || _linkInTextReady) && _target != null;

  Future<void> _add() async {
    if (!_canAdd) return;
    final app = context.app;
    final vault = app.vault;
    final ref = widget.reference;
    // The host outlives this sheet, so the notice and the open action use it.
    final host =
        PageStackScope.of(context).navigator?.overlay?.context ?? context;
    setState(() => _busy = true);
    try {
      final newNote = _target == _newNoteTarget;
      final pageId = newNote ? await vault.createPage() : _target!;
      final page = await vault.loadPage(pageId);
      String? after;
      for (final b in page.blocks) {
        if (b.depth == 0) after = b.id;
      }
      final blockId = newId();
      await vault.apply(pageId, [
        BOp(
          op: OpKind.insert,
          id: blockId,
          after: after,
          kind: Kinds.chatRef,
          text: ref.searchText,
          marks: const [],
          checked: false,
        ),
        // Insert creates the block; its payload is a separate operation.
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
      final name = newNote
          ? 'New note'
          : displayTitle(page.title, firstLine: PageMeta.of(page).firstLine);
      if (mounted) Navigator.of(context).pop();
      if (host.mounted) {
        _showNotice(
          host,
          'Added to ${newNote ? 'a new note' : name}',
          () => openPage(host, pageId, page.title),
        );
      }
    } catch (_) {
      if (mounted) setState(() => _busy = false);
      if (host.mounted) {
        showErrorToast(host, 'Could not add the reference to the note');
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final r = widget.reference;
    final pages = context.watch<PagesStore>();
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 520),
      child: PickerSheet(
        header: Padding(
          padding: const EdgeInsets.fromLTRB(
            Layout.gutter,
            14,
            Layout.gutter,
            8,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Add “${r.title}” to a note',
                style: tsDisplay(ty.h3, c.textPrimary),
              ),
              const SizedBox(height: 2),
              Text(r.label, style: ts(ty.uiSmall, c.textTertiary)),
            ],
          ),
        ),
        above: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            PickerGroup(
              label: 'Show as',
              children: [
                Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: Layout.gutter,
                  ),
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Expanded(
                        child: _ShowAsOption(
                          key: const ValueKey('add-show-card'),
                          name: 'Card',
                          selected: !_asLink,
                          onTap: () => setState(() => _asLink = false),
                          preview: ChatReferenceCard(reference: r),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: _ShowAsOption(
                          key: const ValueKey('add-show-link'),
                          name: 'Link in text',
                          selected: _asLink,
                          enabled: _linkInTextReady,
                          hint: _linkInTextReady ? null : 'Coming later',
                          onTap: () => setState(() => _asLink = true),
                          preview: ChatReferenceChip(reference: r),
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
            const SheetHeader('Into'),
          ],
        ),
        search: PickerSearch(
          controller: _query,
          focusNode: _focus,
          onChanged: _search,
          placeholder: 'Search notes',
          fieldKey: const ValueKey('add-search'),
        ),
        listHeight: 280,
        list: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final n in _notes)
              SheetItem(
                key: ValueKey('add-note-${n.id}'),
                label: _noteLabel(pages, n),
                leading: PageIcon(
                  icon: n.icon,
                  size: 20,
                  color: c.iconSecondary,
                ),
                selected: _target == n.id,
                onTap: () => setState(() => _target = n.id),
              ),
            SheetItem(
              key: const ValueKey('add-note-new'),
              label: 'New note',
              icon: Ico.plus,
              selected: _target == _newNoteTarget,
              onTap: () => setState(() => _target = _newNoteTarget),
            ),
          ],
        ),
        footer: Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            SecondaryButton(
              label: 'Cancel',
              height: 36,
              onTap: _busy ? null : () => Navigator.of(context).pop(),
            ),
            const SizedBox(width: 8),
            PrimaryButton(
              key: const ValueKey('add-go'),
              label: 'Add',
              height: 36,
              onTap: () {
                if (_canAdd) unawaited(_add());
              },
            ),
          ],
        ),
      ),
    );
  }
}

class _ShowAsOption extends StatelessWidget {
  const _ShowAsOption({
    super.key,
    required this.name,
    required this.selected,
    required this.onTap,
    required this.preview,
    this.enabled = true,
    this.hint,
  });
  final String name;
  final bool selected;
  final bool enabled;
  final String? hint;
  final VoidCallback onTap;
  final Widget preview;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return Hover(
      radius: Radii.lg,
      selected: selected,
      onTap: enabled ? onTap : null,
      padding: const EdgeInsets.all(10),
      background: c.bgElevated,
      child: Opacity(
        opacity: enabled ? 1 : .5,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(name, style: ts(ty.ui, c.textPrimary, weight: 600)),
            const SizedBox(height: 6),
            IgnorePointer(
              child: SizedBox(
                height: 76,
                width: double.infinity,
                child: FittedBox(
                  fit: BoxFit.scaleDown,
                  alignment: Alignment.topLeft,
                  child: SizedBox(width: 420, child: preview),
                ),
              ),
            ),
            if (hint != null) ...[
              const SizedBox(height: 6),
              Text(hint!, style: ts(ty.uiSmall, c.textTertiary)),
            ],
          ],
        ),
      ),
    );
  }
}

// ---- notice after adding ---------------------------------------------------

/// A small notice with an Open action, shown on the overlay for a few seconds.
void _showNotice(BuildContext host, String message, VoidCallback onOpen) {
  final overlay = Overlay.maybeOf(host, rootOverlay: true);
  if (overlay == null) return;
  late final OverlayEntry entry;
  var closed = false;
  void close() {
    if (closed) return;
    closed = true;
    entry.remove();
  }

  entry = OverlayEntry(
    builder: (_) => _Notice(
      message: message,
      onOpen: () {
        close();
        onOpen();
      },
      onExpire: close,
    ),
  );
  overlay.insert(entry);
}

class _Notice extends StatefulWidget {
  const _Notice({
    required this.message,
    required this.onOpen,
    required this.onExpire,
  });
  final String message;
  final VoidCallback onOpen;
  final VoidCallback onExpire;

  @override
  State<_Notice> createState() => _NoticeState();
}

class _NoticeState extends State<_Notice> {
  late final Timer _timer = Timer(const Duration(seconds: 5), widget.onExpire);

  @override
  void dispose() {
    _timer.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return Positioned(
      left: 0,
      right: 0,
      bottom: 24,
      child: Center(
        child: Container(
          key: const ValueKey('add-notice'),
          margin: const EdgeInsets.symmetric(horizontal: 16),
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
          decoration: BoxDecoration(
            color: c.rail,
            borderRadius: BorderRadius.circular(Radii.lg),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Flexible(
                child: Text(widget.message, style: ts(ty.ui, c.railInk)),
              ),
              const SizedBox(width: 12),
              Hover(
                key: const ValueKey('add-notice-open'),
                radius: Radii.sm,
                onTap: widget.onOpen,
                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                child: Text(
                  'Open',
                  style: ts(ty.ui, c.accentText, weight: 600),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
