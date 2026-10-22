// Under "Let the hub use this computer" (Settings > Sync, desktop) while it
// is on: the name this computer has on the hub, the relay's connection state
// (read every [HostRelayLink.statusInterval] while shown) and the connector
// address for ChatGPT or claude.ai, with Copy.
//
// The name is the hub's label for this device, from sign-in
// (`clientDeviceLabel`); the hub matches `?host=` against it exactly.

import 'package:flutter/widgets.dart';

import '../../services/addon_services.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';
import 'host_relay_link.dart';

/// `<hub>/mcp/agent?host=<name>`: the connector address that runs this
/// computer's tools through [hubUrl].
String hostConnectorUrl(String hubUrl, String name) {
  var base = hubUrl.trim();
  while (base.endsWith('/')) {
    base = base.substring(0, base.length - 1);
  }
  return '$base/mcp/agent?host=${Uri.encodeComponent(name.trim())}';
}

class HostRelayDetails extends StatefulWidget {
  const HostRelayDetails({
    super.key,
    required this.link,
    required this.hubUrl,
    required this.name,
  });

  final HostRelayLink link;

  /// The signed-in hub's address and this computer's name on it.
  final String hubUrl, name;

  @override
  State<HostRelayDetails> createState() => _HostRelayDetailsState();
}

class _HostRelayDetailsState extends State<HostRelayDetails> {
  @override
  void initState() {
    super.initState();
    widget.link.watchStatus();
  }

  @override
  void didUpdateWidget(HostRelayDetails old) {
    super.didUpdateWidget(old);
    if (!identical(old.link, widget.link)) {
      old.link.unwatchStatus();
      widget.link.watchStatus();
    }
  }

  @override
  void dispose() {
    widget.link.unwatchStatus();
    super.dispose();
  }

  /// The state in words, and whether it is a problem.
  (String, bool) _describe(AgentHostRelayStatus? s) {
    final error = s?.error?.trim();
    final why = error == null || error.isEmpty ? null : error;
    return switch (s?.state) {
      null => ('Checking the connection…', false),
      'connected' => ('Connected to the hub.', false),
      'connecting' => ('Connecting to the hub…', false),
      'retrying' => (
        why == null
            ? "Can't reach the hub. Trying again."
            : "Can't reach the hub: $why. Trying again.",
        true,
      ),
      _ when why != null => ('Not connected: $why', true),
      _ when !widget.hubUrl.trim().toLowerCase().startsWith('https://') => (
        'Not connected. The hub needs an https:// address.',
        true,
      ),
      _ => ('Not connected.', false),
    };
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsRow(
          key: const ValueKey('host-relay-name'),
          label: 'Appears on the hub as ${widget.name}',
          description:
              "This computer's name from when it signed in to the hub.",
        ),
        ValueListenableBuilder<AgentHostRelayStatus?>(
          valueListenable: widget.link.status,
          builder: (context, status, _) {
            final (text, problem) = _describe(status);
            return Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(
                text,
                key: const ValueKey('host-relay-status'),
                style: tsRaw(
                  12,
                  16,
                  400,
                  problem ? c.inlineCodeText : c.textSecondary,
                ),
              ),
            );
          },
        ),
        SettingsRow(
          label: 'Connector address',
          description:
              'Add it as a custom connector in ChatGPT or claude.ai to use '
              'this computer through the hub.',
          stack: true,
          trailing: CopyableText(
            hostConnectorUrl(widget.hubUrl, widget.name),
            key: const ValueKey('host-relay-url'),
            textKey: const ValueKey('host-relay-url-text'),
          ),
        ),
      ],
    );
  }
}
