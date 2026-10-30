// Conflicts: sections and blocks two devices changed before they synced. The
// page is the review. A section or page conflict shows a topline above the
// content (ConflictToplines). A block changed on both devices shows the other
// version under it (ConflictBlocks, the editor's BlockDecor hook). The rule
// has already applied when the conflict is listed; the user only confirms it
// (OK) or takes the other version. Resolving on one device clears it everywhere.
//
// - ConflictToplines: the scope lines above a page's content.
// - ConflictBlocks: the tone and inline review the page's blocks take.
// - ConflictRows: one row per page with pending conflicts, as a settings list,
//   Home cards or rail rows.
// - ConflictHomeSection / ConflictRailGroup: the phone Home section and the
//   desktop rail group the shell mounts; hidden with nothing pending.
// - ConflictLog: the Recent list (resolved, settled and undone entries).
// - ConflictsBody: the settings section with the rows, the auto-resolve
//   switch and Recent.
//
// A block changed on both devices shows one side's text: the core picks it
// when it records the conflict (conflictShownSide states the rule), and the
// labels say whether that is the local or the cloud version.
//
// Where neither side is local (both are other devices), the copy and the
// buttons name the devices. See conflict_copy.dart.

import 'package:flutter/widgets.dart';

import '../../core/change_feed.dart';
import '../../core/models.dart';
import '../../editor/block_row.dart';
import '../../services/legacy_host.dart' show openPage;
import '../../state/app_state.dart';
import '../../state/pages_store.dart' show firstLineOfTexts;
import '../../state/sync_store.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import '../settings/settings_kit.dart';
import 'conflict_copy.dart';
import 'conflict_toast.dart';

/// Opens [pageId], where its conflicts are reviewed. From the settings panel
/// the panel closes too: the page would otherwise open beneath it.
Future<void> showConflicts(BuildContext context, String pageId, {String title = ''}) {
  final nav = Navigator.of(context);
  final panel = ModalRoute.of(context);
  final opening = openPage(context, pageId, title);
  if (panel is PopupRoute) nav.removeRoute(panel);
  return opening;
}

// ---- conflict shape helpers ----

/// The side whose version the page shows (present); null if none.
int? _editingIndex(Conflict c) {
  final i = c.sides.indexWhere((s) => s.state.kind == ConflictStateKind.present);
  return i < 0 ? null : i;
}

/// The side that deleted, trashed or purged the scope; null if none.
int? _otherIndex(Conflict c) {
  final i = c.sides.indexWhere((s) => s.state.kind != ConflictStateKind.present);
  return i < 0 ? null : i;
}

/// The local side, or null when neither side is local.
int? _localIndex(Conflict c) {
  final i = c.sides.indexWhere((s) => s.local);
  return i < 0 ? null : i;
}

/// The side whose text a block changed on both devices shows. The core sets
/// the block to that side's text when it records the conflict: the later edit
/// (`ConflictSide.timeMs`) wins, and equal times go to [_winsTie]. Null if the
/// conflict has no sides.
int? conflictShownSide(Conflict c) {
  if (c.sides.isEmpty) return null;
  var best = 0;
  for (var i = 1; i < c.sides.length; i++) {
    final t = c.sides[i].timeMs.compareTo(c.sides[best].timeMs);
    if (t > 0 || (t == 0 && _winsTie(c.sides[i], c.sides[best]))) best = i;
  }
  return best;
}

/// The tie-break for equal edit times: the greater device id wins (compared
/// as strings; device ids are ASCII, so this matches the core's byte order).
bool _winsTie(ConflictSide a, ConflictSide b) => a.device.compareTo(b.device) > 0;

/// The side a block review offers: the one the note does not show. Null if
/// the conflict has fewer than two sides.
int? _blockOtherIndex(Conflict c) {
  final shown = conflictShownSide(c);
  if (shown == null || c.sides.length < 2) return null;
  return shown == 0 ? 1 : 0;
}

/// Text of the section's heading, from the side the page shows.
String _headingOf(Conflict c) {
  final e = _editingIndex(c);
  final root = c.scope.block;
  if (e == null || root == null) return '';
  for (final b in c.sides[e].state.content) {
    if (b.id == root) return b.text;
  }
  return '';
}

/// Key part shared by a conflict's buttons: the section root or 'page'.
String _scopeKey(Conflict c) => c.scope.kind == ConflictScopeKind.page ? 'page' : (c.scope.block ?? '');

String _sideLabel(SyncStore sync, Conflict c, int i) {
  final s = c.sides[i];
  if (s.local) return conflictLocalLabel;
  // Neither side is local: name the device instead.
  if (_localIndex(c) == null) return sync.deviceName(s.device);
  return conflictCloudLabel(sync.deviceName(s.device));
}

/// The device of side [i]; "Another device" when there is no such side.
String _deviceOf(SyncStore sync, Conflict c, int? i) => sync.deviceName(i == null ? '' : c.sides[i].device);

Future<void> _resolve(BuildContext context, Conflict c, Resolution r, {String? message, bool undo = false}) async {
  final sync = context.read<SyncStore>();
  final overlay = Overlay.maybeOf(context);
  final area = overlay == null ? EdgeInsets.zero : _pageArea(context, overlay);
  await sync.resolve(c, r);
  // The page may have closed while the change was written.
  if (message == null || overlay == null || !overlay.mounted) return;
  showConflictToast(
    overlay,
    message,
    area: area,
    undo: undo ? () => sync.undoConflict(c) : null,
    onError: (e) => FlutterError.reportError(FlutterErrorDetails(exception: e)),
  );
}

/// How far the page sheet sits from [overlay]'s left and right edges, so the
/// toast centres on the sheet (the desktop sidebar is outside it). The sheet
/// is the widest box around [context] that is narrower than the overlay; on a
/// phone there is none and the toast spans the window.
EdgeInsets _pageArea(BuildContext context, OverlayState overlay) {
  final box = overlay.context.findRenderObject();
  if (box is! RenderBox || !box.hasSize) return EdgeInsets.zero;
  RenderBox? sheet;
  for (RenderObject? n = context.findRenderObject(); n != null && n != box; n = n.parent) {
    if (n is! RenderBox || !n.hasSize) continue;
    if (n.size.width >= box.size.width) break;
    sheet = n;
  }
  if (sheet == null) return EdgeInsets.zero;
  final left = sheet.localToGlobal(Offset.zero, ancestor: box).dx;
  final right = box.size.width - left - sheet.size.width;
  return EdgeInsets.only(left: left.clamp(0, box.size.width), right: right.clamp(0, box.size.width));
}

Resolution _keep() => const Resolution(kind: ResolutionKind.keep, side: 0);

Resolution _useSide(int i) => Resolution(kind: ResolutionKind.useSide, side: i);

/// "Use the local version", "Use the cloud version", or the device's name
/// when neither side is local.
String _useLabel(SyncStore sync, Conflict c, int i) {
  if (c.sides[i].local) return conflictUseLocal;
  if (_localIndex(c) == null) return conflictUseDevice(_deviceOf(sync, c, i));
  return conflictUseCloud;
}

/// The toast after a block takes side [i].
String _usedLine(SyncStore sync, Conflict c, int i) {
  if (_localIndex(c) == null) return conflictUsedDeviceLine(_deviceOf(sync, c, i));
  return c.sides[i].local ? conflictUsedLocalLine : conflictUsedCloudLine;
}

/// The copy above a section or page conflict. It names "the local version" and
/// "the cloud version" when one side is local, and the devices otherwise.
String _scopeCopy(SyncStore sync, Conflict k) {
  final editing = _editingIndex(k);
  final other = _otherIndex(k);
  final localEdit = editing != null && k.sides[editing].local;
  final named = _localIndex(k) == null;
  final editor = _deviceOf(sync, k, editing);
  final deleter = _deviceOf(sync, k, other);
  if (k.scope.kind == ConflictScopeKind.subtree) {
    if (localEdit) return sectionBackLocalCopy;
    return named ? sectionBackNamedCopy(editor, deleter) : sectionBackCloudCopy;
  }
  final purged = other != null && k.sides[other].state.kind == ConflictStateKind.purged;
  if (localEdit) return purged ? pagePurgedLocalCopy : pageTrashedLocalCopy;
  if (named) return purged ? pagePurgedNamedCopy(editor, deleter) : pageTrashedNamedCopy(editor, deleter);
  return purged ? pagePurgedCopy : pageTrashedCopy;
}

// ---- the page toplines ----

/// The scope lines above a page's content: one per pending section or page
/// conflict. A block conflict has no topline; its review sits under the block.
class ConflictToplines extends StatelessWidget {
  const ConflictToplines({super.key, required this.pageId});
  final String pageId;

  @override
  Widget build(BuildContext context) {
    final list = context
        .watch<SyncStore>()
        .conflictsOf(pageId)
        .where((c) => c.scope.kind != ConflictScopeKind.block)
        .toList();
    if (list.isEmpty) return const SizedBox.shrink();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [for (final c in list) _Topline(conflict: c)],
    );
  }
}

class _Topline extends StatelessWidget {
  const _Topline({required this.conflict});
  final Conflict conflict;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final sync = context.read<SyncStore>();
    final k = conflict;
    final key = _scopeKey(k);
    final other = _otherIndex(k);
    final isPage = k.scope.kind == ConflictScopeKind.page;

    final acts = isPage
        ? [
            // No toast: the topline already says the page is back in Pages.
            TextBtn(conflictOk,
                key: ValueKey('keep-$key'),
                onTap: () => _resolve(context, k, _keep()),
                color: c.accentText),
            if (other != null)
              // Mirrored (this device kept it, the cloud deleted it): the
              // other version is the cloud's, not a second delete.
              TextBtn(k.sides[other].local || _localIndex(k) == null ? conflictDeleteAgain : _useLabel(sync, k, other),
                  key: ValueKey('use-$key-$other'),
                  onTap: () => _resolve(context, k, _useSide(other), message: conflictDeletedAgain),
                  color: c.accentText),
          ]
        : [
            TextBtn(conflictOk,
                key: ValueKey('keep-$key'),
                onTap: () => _resolve(context, k, _keep()),
                color: c.accentText),
            if (other != null)
              TextBtn(_useLabel(sync, k, other),
                  key: ValueKey('use-$key-$other'),
                  onTap: () => _resolve(context, k, _useSide(other),
                      message: conflictSectionRemoved(_headingOf(k)), undo: true),
                  color: c.accentText),
          ];

    final text = Text(_scopeCopy(sync, k), style: tsRaw(14, 20, 400, c.textSecondary));
    final actions = Row(mainAxisSize: MainAxisSize.min, children: [
      for (var i = 0; i < acts.length; i++) ...[
        if (i > 0) const SizedBox(width: 16),
        acts[i],
      ],
    ]);
    // Desktop: the text takes the row and the actions sit at its end. Phone:
    // the text wraps above the actions, which sit at the right.
    return Container(
      key: ValueKey('conflict-banner-${k.id}'),
      padding: m
          ? const EdgeInsets.symmetric(horizontal: 18, vertical: 8)
          : const EdgeInsets.symmetric(horizontal: 24, vertical: 10),
      decoration: BoxDecoration(color: c.bgSecondary, border: Border(bottom: BorderSide(color: c.borderSubtle))),
      child: m
          ? Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
              text,
              const SizedBox(height: 2),
              Align(alignment: Alignment.centerRight, child: actions),
            ])
          : Row(crossAxisAlignment: CrossAxisAlignment.center, children: [
              Expanded(child: text),
              const SizedBox(width: 16),
              actions,
            ]),
    );
  }
}

// ---- the editor hook ----

/// The editor's block hook for one page: the tint behind a section the cloud
/// restored, and the review under a block changed on both devices. It indexes
/// the page's conflicts once per change of [SyncStore.conflicts].
class ConflictBlocks implements BlockDecor {
  ConflictBlocks({required this.sync, required this.pageId});
  final SyncStore sync;
  final String pageId;

  List<Conflict>? _indexed;
  final _restored = <String>{};
  final _blocks = <String, Conflict>{};

  void _index() {
    final all = sync.conflicts;
    if (identical(all, _indexed)) return;
    _indexed = all;
    _restored.clear();
    _blocks.clear();
    for (final c in all) {
      if (c.page != pageId) continue;
      if (c.scope.kind == ConflictScopeKind.block) {
        final b = c.scope.block;
        if (b != null) _blocks[b] = c;
      } else if (c.scope.kind == ConflictScopeKind.subtree) {
        final e = _editingIndex(c);
        if (e != null) _restored.addAll(c.sides[e].state.content.map((b) => b.id));
      }
    }
  }

  @override
  void addListener(VoidCallback listener) => sync.addListener(listener);

  @override
  void removeListener(VoidCallback listener) => sync.removeListener(listener);

  @override
  Color? tone(BuildContext context, String id) {
    _index();
    return _restored.contains(id) ? context.colors.okBg : null;
  }

  @override
  Widget? above(BuildContext context, String id) {
    _index();
    final c = _blocks[id];
    return c == null ? null : _MergedLabel(conflict: c);
  }

  @override
  Widget? below(BuildContext context, String id) {
    _index();
    final c = _blocks[id];
    return c == null ? null : _OtherVersion(conflict: c);
  }
}

class _MergedLabel extends StatelessWidget {
  const _MergedLabel({required this.conflict});
  final Conflict conflict;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final sync = context.read<SyncStore>();
    // The note shows the winning side's text: "Local", "Cloud · from <device>",
    // or the device's name when neither side is local.
    final shown = conflictShownSide(conflict);
    if (shown == null) return const SizedBox.shrink();
    final label = _sideLabel(sync, conflict, shown);
    return Padding(
      padding: const EdgeInsets.only(top: 4, bottom: 2),
      child: Text(label, style: tsRaw(12, 16, 500, c.textTertiary)),
    );
  }
}

class _OtherVersion extends StatelessWidget {
  const _OtherVersion({required this.conflict});
  final Conflict conflict;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final sync = context.read<SyncStore>();
    final k = conflict;
    final alt = _blockOtherIndex(k);
    if (alt == null) return const SizedBox.shrink();
    final text = k.sides[alt].block?.text;
    final id = k.scope.block ?? '';
    return Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Expanded(
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text(_sideLabel(sync, k, alt), style: tsRaw(12, 16, 500, c.textTertiary)),
          if (text != null)
            GestureDetector(
              key: ValueKey('use-$id-$alt'),
              behavior: HitTestBehavior.opaque,
              onTap: () => _resolve(context, k, _useSide(alt), message: _usedLine(sync, k, alt), undo: true),
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 2),
                child: Text(text.isEmpty ? conflictEmptyBlock : text,
                    style: m ? tsRaw(16, 22, 400, c.textTertiary) : tsRaw(16, 24, 400, c.textTertiary)),
              ),
            ),
        ]),
      ),
      const SizedBox(width: 8),
      TextBtn(conflictOk,
          key: ValueKey('keep-$id'),
          onTap: () => _resolve(context, k, _keep()),
          color: c.accentText),
    ]);
  }
}

// ---- the page list (reusable) ----

/// Titles and first lines of pages named by conflicts, loaded once per page.
mixin _PageTitles<T extends StatefulWidget> on State<T> {
  final _titles = <String, String>{};
  final _lines = <String, String>{};

  String titleOf(String page) {
    if (!_titles.containsKey(page)) {
      _titles[page] = '';
      context.app.vault.loadPage(page).then((p) {
        if (mounted) {
          setState(() {
            _titles[page] = p.title;
            _lines[page] = firstLineOfTexts(p.blocks.map((b) => b.text));
          });
        }
      }, onError: (_) {});
    }
    return displayTitle(_titles[page] ?? '', firstLine: _lines[page] ?? '');
  }
}

/// How [ConflictRows] draws: the settings list, the Home cards, or the
/// desktop rail.
enum ConflictRowsLook { list, card, rail }

/// The small dot before a row. Apricot for every row: the mockup's per-space
/// dots would need the page's space.
Widget _dot(BuildContext context) {
  final c = context.colors;
  return Container(
    width: 8,
    height: 8,
    decoration: BoxDecoration(
      color: spaceColor('apricot', c.dark ? Brightness.dark : Brightness.light),
      borderRadius: BorderRadius.circular(2),
    ),
  );
}

/// The count of pages with pending conflicts, as a pill.
class ConflictCount extends StatelessWidget {
  const ConflictCount(this.count, {super.key});
  final int count;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final patch = spacePatch('apricot', c.dark ? Brightness.dark : Brightness.light);
    return Container(
      constraints: const BoxConstraints(minWidth: 20),
      padding: const EdgeInsets.symmetric(horizontal: 7),
      decoration: BoxDecoration(color: patch?.fill ?? c.accent, borderRadius: BorderRadius.circular(999)),
      child: Text('$count', textAlign: TextAlign.center, style: tsRaw(12, 16, 600, patch?.ink ?? c.onAccent)),
    );
  }
}

/// One row per page with pending conflicts. A row opens the page, where the
/// review is. Settings shows it as a list, phone Home as cards
/// ([ConflictHomeSection]) and the desktop rail as a group ([ConflictRailGroup]).
class ConflictRows extends StatefulWidget {
  const ConflictRows({super.key, this.look = ConflictRowsLook.list});
  final ConflictRowsLook look;

  @override
  State<ConflictRows> createState() => _ConflictRowsState();
}

class _ConflictRowsState extends State<ConflictRows> with _PageTitles {
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final sync = context.watch<SyncStore>();
    final pages = sync.conflictPages;
    if (pages.isEmpty) {
      if (widget.look != ConflictRowsLook.list) return const SizedBox.shrink();
      return const SEmpty(
        key: ValueKey('no-conflicts'),
        icon: Sx.circleCheck,
        title: conflictsNoneTitle,
        text: conflictsNone,
      );
    }
    final rows = [for (final p in pages) _row(context, sync, p, sync.conflictsOf(p).first)];
    if (widget.look == ConflictRowsLook.list) return SList(children: rows);
    if (widget.look != ConflictRowsLook.card) {
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: rows);
    }
    return Container(
      decoration: BoxDecoration(
        color: c.bgPrimary,
        border: Border.all(color: c.border),
        borderRadius: BorderRadius.circular(12),
      ),
      clipBehavior: Clip.antiAlias,
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        for (var i = 0; i < rows.length; i++) ...[
          if (i > 0) Container(height: 1, color: c.borderSubtle),
          rows[i],
        ],
      ]),
    );
  }

  Widget _row(BuildContext context, SyncStore sync, String p, Conflict first) {
    final c = context.colors;
    final title = titleOf(p);
    final summary = _summary(sync, first);
    final when = relativeTime(first.timeMs.toInt());
    void open() => showConflicts(context, p, title: title);
    switch (widget.look) {
      case ConflictRowsLook.list:
        return Hover(
          key: ValueKey('conflict-page-$p'),
          onTap: open,
          radius: 0,
          lift: 0,
          child: SItem(
            icon: Sx.fileText,
            name: title,
            sub: summary,
            trailing: [
              Text(when, style: sHint(context)),
              const SizedBox(width: 8),
              SButton('Review', onTap: open),
            ],
          ),
        );
      case ConflictRowsLook.card:
        return Hover(
          key: ValueKey('conflict-home-page-$p'),
          onTap: open,
          radius: 0,
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 32),
            child: Row(children: [
              _dot(context),
              const SizedBox(width: 12),
              Expanded(
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
                  Text(title, maxLines: 1, overflow: TextOverflow.ellipsis, style: tsRaw(15, 20, 500, c.textPrimary)),
                  const SizedBox(height: 2),
                  Text('$summary · $when',
                      maxLines: 1, overflow: TextOverflow.ellipsis, style: tsRaw(12.5, 17, 400, c.textTertiary)),
                ]),
              ),
              const SizedBox(width: 12),
              Icon20(Ico.chevronRight, size: 16, color: c.iconTertiary),
            ]),
          ),
        );
      case ConflictRowsLook.rail:
        return Hover(
          key: ValueKey('conflict-rail-page-$p'),
          onTap: open,
          borderRadius: Radii.ctl,
          hoverColor: c.rail2.withValues(alpha: .55),
          padding: const EdgeInsets.fromLTRB(12, 6, 8, 6),
          child: Row(children: [
            _dot(context),
            const SizedBox(width: 10),
            Expanded(
              child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
                Text(title, maxLines: 1, overflow: TextOverflow.ellipsis, style: tsRaw(14, 20, 500, c.railInk)),
                Text('$summary · $when',
                    maxLines: 1, overflow: TextOverflow.ellipsis, style: tsRaw(12, 16, 400, c.railMuted)),
              ]),
            ),
          ]),
        );
    }
  }
}

// ---- Home and rail entry points ----

/// Phone Home: "Changed on two devices" with a count, then the pages as
/// cards. Nothing when no conflicts are pending. The shell mounts it.
class ConflictHomeSection extends StatelessWidget {
  const ConflictHomeSection({super.key});

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final count = context.watch<SyncStore>().conflictPages.length;
    if (count == 0) return const SizedBox.shrink();
    return Padding(
      key: const ValueKey('conflict-home'),
      padding: const EdgeInsets.fromLTRB(18, 22, 18, 0),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
        Row(children: [
          Expanded(child: Text(conflictsSectionTitle, style: tsRaw(13, 16, 500, c.textSecondary))),
          ConflictCount(count, key: const ValueKey('conflict-home-count')),
        ]),
        const SizedBox(height: 8),
        const ConflictRows(look: ConflictRowsLook.card),
      ]),
    );
  }
}

/// Desktop rail: a collapsible "Changed on two devices" group with a count,
/// above Settings and Trash. Nothing when no conflicts are pending. The shell
/// mounts it.
class ConflictRailGroup extends StatefulWidget {
  const ConflictRailGroup({super.key});

  @override
  State<ConflictRailGroup> createState() => _ConflictRailGroupState();
}

class _ConflictRailGroupState extends State<ConflictRailGroup> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final count = context.watch<SyncStore>().conflictPages.length;
    if (count == 0) return const SizedBox.shrink();
    return Container(
      key: const ValueKey('conflict-rail'),
      margin: const EdgeInsets.only(top: 4),
      padding: const EdgeInsets.fromLTRB(8, 8, 8, 4),
      decoration: BoxDecoration(border: Border(top: BorderSide(color: c.railLine))),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
        Hover(
          key: const ValueKey('conflict-rail-toggle'),
          onTap: () => setState(() => _open = !_open),
          borderRadius: Radii.ctl,
          hoverColor: c.rail2.withValues(alpha: .55),
          padding: const EdgeInsets.symmetric(horizontal: 8),
          child: SizedBox(
            height: Layout.sidebarRowHeight - 2,
            child: Row(children: [
              _dot(context),
              const SizedBox(width: 10),
              Expanded(
                child: Text(conflictsSectionTitle,
                    maxLines: 1, overflow: TextOverflow.ellipsis, style: tsRaw(14, 20, 400, c.railInk)),
              ),
              const SizedBox(width: 8),
              ConflictCount(count, key: const ValueKey('conflict-rail-count')),
            ]),
          ),
        ),
        if (_open)
          ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 240),
            child: const SingleChildScrollView(
              padding: EdgeInsets.only(top: 4, bottom: 6),
              child: ConflictRows(look: ConflictRowsLook.rail),
            ),
          ),
      ]),
    );
  }
}

// ---- review log ----

/// When a log entry last changed: its latest application, or its undo.
int _logTime(Conflict c) {
  if (c.applied.isEmpty) return c.timeMs.toInt();
  final last = c.applied.last;
  return (last.undoneAt ?? last.timeMs).toInt();
}

/// What happened to a log entry, from its latest application.
String conflictLogWhat(Conflict c) {
  if (c.applied.isEmpty) return conflictLogResolved;
  final last = c.applied.last;
  if (last.undoneAt != null) return conflictLogUndone;
  if (last.by == ResolutionBy.auto) return conflictLogAuto;
  final choice = last.resolution;
  if (choice == null) return conflictLogResolved;
  if (choice.kind == ResolutionKind.keep) return conflictLogKept;
  if (choice.side < 0 || choice.side >= c.sides.length) return conflictLogResolved;
  return c.sides[choice.side].local ? conflictLogUsedLocal : conflictLogUsedCloud;
}

/// Settings > Conflicts, Recent: resolved, settled and undone entries from
/// the core's 30-day log, newest first. A row opens its page. Undo shows on a
/// settled entry the core can still undo; it returns the entry to the list
/// above. An undone entry has nothing left to undo here.
class ConflictLog extends StatefulWidget {
  const ConflictLog({super.key});

  @override
  State<ConflictLog> createState() => _ConflictLogState();
}

class _ConflictLogState extends State<ConflictLog> with _PageTitles {
  @override
  Widget build(BuildContext context) {
    final sync = context.watch<SyncStore>();
    final log = [...sync.conflictLog]..sort((a, b) => _logTime(b).compareTo(_logTime(a)));
    if (log.isEmpty) {
      return Padding(
        padding: const EdgeInsets.only(top: 10),
        child: Text(conflictsRecentNone, key: const ValueKey('no-conflict-log'), style: sDesc(context)),
      );
    }
    return Padding(
      padding: const EdgeInsets.only(top: 10),
      child: SList(children: [
        for (final k in log)
          Hover(
            key: ValueKey('conflict-log-${k.page}-${k.id}'),
            onTap: () => showConflicts(context, k.page, title: titleOf(k.page)),
            radius: 0,
            lift: 0,
            child: SItem(
              icon: Sx.history,
              name: titleOf(k.page),
              sub: conflictLogDetail(conflictLogWhat(k), relativeTime(_logTime(k))),
              trailing: [
                if (k.undoable && k.status == ConflictStatus.resolved)
                  SButton(
                    conflictUndo,
                    key: ValueKey('conflict-log-undo-${k.id}'),
                    kind: SButtonKind.quiet,
                    icon: Sx.undo,
                    onTap: () => sync.undoConflict(k),
                  ),
              ],
            ),
          ),
      ]),
    );
  }
}

/// The one-line summary of a conflict in the rows.
String _summary(SyncStore sync, Conflict c) {
  final heading = _headingOf(c);
  final editing = _editingIndex(c);
  final other = _otherIndex(c);
  final localEdit = editing != null && c.sides[editing].local;
  final named = _localIndex(c) == null;
  final editor = _deviceOf(sync, c, editing);
  final purged = other != null && c.sides[other].state.kind == ConflictStateKind.purged;
  switch (c.scope.kind) {
    case ConflictScopeKind.block:
      return conflictSameLine;
    case ConflictScopeKind.subtree:
      if (localEdit) return conflictLocalBroughtBack(heading);
      return named ? conflictDeviceBroughtBack(editor, heading) : conflictCloudBroughtBack(heading);
    case ConflictScopeKind.page:
      if (localEdit) return purged ? conflictLocalBroughtBackPurged : conflictLocalBroughtFromTrash;
      if (named) return purged ? conflictDeviceBroughtBackPurged(editor) : conflictDeviceBroughtFromTrash(editor);
      return purged ? conflictCloudBroughtBackPurged : conflictBroughtFromTrash;
  }
}

// ---- settings ----

/// The settings section: the pages with pending conflicts, then the switch
/// that lets a cloud change settle itself. The switch reads and writes the
/// vault's setting, not AppSettings. It re-reads when the vault changes or a
/// sync finishes, so a change made on another device shows while this is open.
class ConflictsBody extends StatefulWidget {
  const ConflictsBody({super.key});

  @override
  State<ConflictsBody> createState() => _ConflictsBodyState();
}

class _ConflictsBodyState extends State<ConflictsBody> {
  bool? _auto;

  /// Switch writes in flight. A read does not apply while one runs.
  int _writes = 0;

  /// Bumped by each switch change; a read started before it is dropped.
  int _gen = 0;

  late final ChangeFeed _feed = context.app.feed;
  late final SyncStore _sync = context.read<SyncStore>();

  @override
  void initState() {
    super.initState();
    _sync.refreshConflicts();
    _feed.addListener(_onFeed);
    _sync.addListener(_onSync);
    _load();
  }

  @override
  void dispose() {
    _feed.removeListener(_onFeed);
    _sync.removeListener(_onSync);
    super.dispose();
  }

  void _onFeed(ChangeBatch _) => _load();

  void _onSync() => _load();

  Future<void> _load() async {
    final vault = context.app.vault;
    if (_writes > 0 || vault.isClosed) return;
    final gen = _gen;
    try {
      final on = await vault.run((v) => v.conflictAutoResolve());
      if (!mounted || _writes > 0 || gen != _gen || on == _auto) return;
      setState(() => _auto = on);
    } catch (_) {}
  }

  Future<void> _set(bool on) async {
    final before = _auto;
    if (before == null) return;
    final vault = context.app.vault;
    _gen++;
    _writes++;
    setState(() => _auto = on);
    try {
      await vault.run((v) => v.setConflictAutoResolve(enabled: on), writes: true);
    } catch (_) {
      if (mounted) {
        setState(() => _auto = before);
        showErrorToast(context, conflictAutoResolveSaveFailed);
      }
    } finally {
      _writes--;
    }
  }

  @override
  Widget build(BuildContext context) {
    return SPage(
      title: conflictsSettingsTitle,
      intro: conflictsIntro,
      children: [
        const SSection(title: 'Needs a look', first: true, children: [ConflictRows()]),
        SSection(
          title: 'Handling',
          children: [
            SRow(
              label: conflictAutoResolveName,
              desc: conflictAutoResolveDesc,
              help: conflictAutoResolveHelp,
              scope: 'All devices',
              control: SSwitch(
                key: const ValueKey('conflict-auto-resolve'),
                label: conflictAutoResolveName,
                value: _auto ?? false,
                onChanged: _auto == null ? null : _set,
              ),
            ),
          ],
        ),
        const SSection(
          key: ValueKey('conflict-log'),
          title: conflictsRecentTitle,
          children: [ConflictLog()],
        ),
      ],
    );
  }
}
