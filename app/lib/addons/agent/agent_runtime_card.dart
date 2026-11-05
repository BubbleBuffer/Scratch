// The download card above the agent pane's composer. Claude Code is a
// runtime package (wiki/addons/packages.md) fetched when the user asks, so
// when the chat's model comes from it and it is missing, the card offers it:
//
//   idle      "Claude Code", the size, Download
//   progress  the percent while downloading, then Verifying / Extracting /
//             Installing
//   ready     "Ready" while the agent looks for it again, then the card goes
//   error     the message and Retry
//
// An update while Claude Code still works is one compact line ("Update
// available", the size, Update); the old version keeps working until the new
// one is in place. Nothing downloads without a tap.

import 'package:flutter/widgets.dart';

import '../../services/addon_services.dart' show AgentHost;
import '../../ui/ui.dart';
import 'agent_store.dart';
import 'packages_store.dart';

/// How much of the card shows.
enum RuntimeCardMode { hidden, block, compact }

/// The provider of a model id (`claude-code/claude-opus-5-5` is `claude-code`).
String? modelProvider(String? model) {
  if (model == null) return null;
  final i = model.indexOf('/');
  return i <= 0 ? null : model.substring(0, i);
}

/// Whether and how the card shows.
///
/// [selected]: the chat's model (or the default one) comes from Claude Code.
/// [available]: the host reports the provider; null while it is not known
/// (the agent is opening). [ready]: the package was installed a moment ago.
RuntimeCardMode runtimeCardMode({
  required bool selected,
  required bool? available,
  required PackageInfo? info,
  required PackageStatus status,
  required bool ready,
}) {
  if (!selected) return RuntimeCardMode.hidden;
  RuntimeCardMode shown() =>
      available == true ? RuntimeCardMode.compact : RuntimeCardMode.block;
  if (status is PackageInstalling) return shown();
  if (ready && available != true) return RuntimeCardMode.block;
  if (available == null || info == null) return RuntimeCardMode.hidden;
  final offered =
      info.state == PackageState.notInstalled ||
      info.state == PackageState.updateAvailable;
  if (!offered) return RuntimeCardMode.hidden;
  // Working Claude Code with nothing newer: no card. A failed update stays
  // visible so Retry is reachable.
  if (available) {
    return info.state == PackageState.updateAvailable
        ? RuntimeCardMode.compact
        : RuntimeCardMode.hidden;
  }
  return RuntimeCardMode.block;
}

class AgentRuntimeCard extends StatefulWidget {
  const AgentRuntimeCard({super.key, required this.store});
  final AgentStore store;

  @override
  State<AgentRuntimeCard> createState() => _AgentRuntimeCardState();
}

class _AgentRuntimeCardState extends State<AgentRuntimeCard> {
  @override
  void initState() {
    super.initState();
    widget.store.packages.ensureLoaded();
  }

  @override
  Widget build(BuildContext context) {
    final store = widget.store;
    return ListenableBuilder(
      listenable: Listenable.merge([store, store.packages]),
      builder: (context, _) {
        if (store.host != AgentHost.local) return const SizedBox.shrink();
        final info = store.hostInfo;
        return RuntimePackageCard(
          packages: store.packages,
          selected:
              modelProvider(store.modelOf(store.openId)) == claudeCodeProvider,
          available: info?.providers.contains(claudeCodeProvider),
        );
      },
    );
  }
}

/// The provider id Claude Code's models carry.
const claudeCodeProvider = 'claude-code';

/// The card for the Claude Code package, from the facts that decide it.
class RuntimePackageCard extends StatelessWidget {
  const RuntimePackageCard({
    super.key,
    required this.packages,
    required this.selected,
    required this.available,
  });
  final PackagesStore packages;
  final bool selected;
  final bool? available;

  @override
  Widget build(BuildContext context) {
    const name = claudeCodePackage;
    final info = packages.infoOf(name);
    final status = packages.statusOf(name);
    final mode = runtimeCardMode(
      selected: selected,
      available: available,
      info: info,
      status: status,
      ready: packages.justInstalled(name),
    );
    if (mode == RuntimeCardMode.hidden) return const SizedBox.shrink();
    final c = context.colors;
    final size = info == null ? null : packageSize(info.downloadBytes.toInt());
    final update = info?.state == PackageState.updateAvailable;
    final compact = mode == RuntimeCardMode.compact;
    final working = status is PackageInstalling;
    final ready =
        !working && status is! PackageFailed && packages.justInstalled(name);
    final failed = status is PackageFailed ? status.message : null;

    final String line;
    if (status is PackageInstalling) {
      line = _progressLine(status, update);
    } else if (failed != null) {
      line = failed;
    } else if (ready) {
      line = 'Ready';
    } else if (compact) {
      line = 'Update available${size == null ? '' : ' · $size'}';
    } else {
      line = size ?? '';
    }
    const title = 'Claude Code';
    final action = working || ready
        ? null
        : failed != null
        ? 'Retry'
        : update
        ? 'Update'
        : 'Download';
    final Widget? button = action == null
        ? null
        : PrimaryButton(
            key: const ValueKey('agent-runtime-action'),
            label: action,
            height: 32,
            onTap: () => packages.install(name),
          );
    final text = Text(
      line,
      key: const ValueKey('agent-runtime-status'),
      maxLines: failed != null ? 3 : 1,
      overflow: TextOverflow.ellipsis,
      style: ts(
        context.type.uiSmall,
        failed != null ? c.textPrimary : c.textSecondary,
      ),
    );
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Container(
      key: ValueKey(
        compact ? 'agent-runtime-card-compact' : 'agent-runtime-card',
      ),
      padding: EdgeInsets.symmetric(horizontal: 12, vertical: compact ? 6 : 10),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: Radii.panel,
        border: Border.all(color: failed != null ? c.dangerBg : c.border),
      ),
      child: Row(
        children: [
          Expanded(
            child: compact
                ? Row(
                    children: [
                      Text(title, style: tsRaw(14, 19, 680, c.textPrimary)),
                      const SizedBox(width: 8),
                      Expanded(child: text),
                    ],
                  )
                : Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(title, style: tsRaw(14, 19, 680, c.textPrimary)),
                      const SizedBox(height: 2),
                      text,
                    ],
                  ),
          ),
          if (working) ...[
            const SizedBox(width: 8),
            Spinner(size: 14, color: c.iconTertiary),
          ],
          if (button != null) ...[const SizedBox(width: 12), button],
        ],
      ),
    ),
    );
  }
}

/// "Downloading 42%", then the step it is on. [update] says "Updating".
String _progressLine(PackageInstalling p, bool update) {
  switch (p.phase) {
    case PackagePhase.downloading:
      return '${update ? 'Updating' : 'Downloading'} ${p.percent}%';
    case PackagePhase.verifying:
      return 'Verifying';
    case PackagePhase.extracting:
      return 'Extracting';
    case PackagePhase.installing:
      return 'Installing';
  }
}
