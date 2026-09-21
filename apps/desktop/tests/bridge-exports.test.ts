import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

async function source(file: URL): Promise<ts.SourceFile> {
  return ts.createSourceFile(
    file.pathname,
    await readFile(file, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  );
}

async function exportsOf(file: URL): Promise<Map<string, 'type' | 'value'>> {
  const result = new Map<string, 'type' | 'value'>();
  for (const node of (await source(file)).statements) {
    if (ts.isExportDeclaration(node)) {
      assert.ok(
        node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier),
      );
      const target = await exportsOf(
        new URL(`${node.moduleSpecifier.text}.ts`, file),
      );
      if (!node.exportClause) {
        for (const [name, kind] of target) result.set(name, kind);
      } else {
        assert.ok(ts.isNamedExports(node.exportClause));
        for (const entry of node.exportClause.elements) {
          const name = (entry.propertyName ?? entry.name).text;
          assert.ok(
            target.has(name),
            `${file.pathname}: missing export ${name}`,
          );
          result.set(
            entry.name.text,
            node.isTypeOnly || entry.isTypeOnly ? 'type' : target.get(name)!,
          );
        }
      }
      continue;
    }
    if (
      !ts.canHaveModifiers(node) ||
      !ts
        .getModifiers(node)
        ?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword)
    )
      continue;
    if (ts.isInterfaceDeclaration(node) || ts.isTypeAliasDeclaration(node))
      result.set(node.name.text, 'type');
    else if (ts.isFunctionDeclaration(node)) {
      assert.ok(node.name);
      result.set(node.name.text, 'value');
    } else if (ts.isVariableStatement(node)) {
      for (const declaration of node.declarationList.declarations) {
        assert.ok(ts.isIdentifier(declaration.name));
        result.set(declaration.name.text, 'value');
      }
    } else assert.fail(`Unexpected public declaration in ${file.pathname}`);
  }
  return result;
}

const publicTypes = [
  'CommandError',
  'MaintenanceKind',
  'MaintenancePhase',
  'MaintenanceOperationOutcome',
  'MaintenanceDisposition',
  'MaintenanceSnapshot',
  'GroupDetailResult',
  'GroupDetailsDto',
  'RoleDto',
  'StoreDto',
  'ItemDto',
  'CatalogInventoryDto',
  'CatalogDto',
  'AccountDto',
  'CatalogFailureDto',
  'ItemRequest',
  'ReadItemResponse',
  'CopyResponse',
  'DownloadResponse',
  'MutationResponse',
  'KvRoleInput',
  'CreateRoleRequest',
  'CreateTextRequest',
  'CreateLinkRequest',
  'CreateFolderRequest',
  'CreateFileRequest',
  'EditTextRequest',
  'RemoveItemRequest',
  'ImportDroppedFileRequest',
  'ReplaceDroppedFileRequest',
  'DropHoverEvent',
  'ExitAction',
  'ExitState',
  'WindowStateEvent',
  'CreateGroupRequest',
  'GroupMemberRequest',
  'GroupRoleRequest',
  'AdmitGroupRequest',
  'RemoveFederatedGroupRequest',
  'CheckedProfileResponse',
  'PendingOperationKind',
  'PendingOperation',
  'FirstRunAccountRequest',
  'TrackedAccountAttempt',
  'TrackedAccountRequest',
  'FirstRunPassphraseRequest',
  'BackupPhraseResponse',
  'DiscoveredGroup',
  'GroupDiscoveryResponse',
  'StoredHost',
  'ServerStatusSnapshot',
  'ServerLabelResponse',
  'AgentProcessInfo',
  'AppInfo',
  'GoProfileCandidate',
  'GoProfileDiscovery',
  'AppLockState',
  'ServerVersionInfo',
  'CheckedServer',
  'AccountDevice',
  'BackupEnrollment',
  'BackupRevocation',
  'PairingOffer',
  'DeviceProvision',
  'PassphraseReport',
  'PassphraseStatus',
  'ResetPreview',
  'YubiEnrollment',
  'CreateYubiAccountRequest',
  'ProvisionYubiDeviceRequest',
  'YubiCommand',
  'FirstRunFixturePath',
  'FirstRunFixture',
  'Unlisten',
  'CommandAck',
  'Bridge',
];

const publicValues = [
  'commandRecovery',
  'isAgentSessionError',
  'isTerminalCommandError',
  'normalizeMutationError',
  'onAgentReadinessRequired',
  'isAgentReadinessError',
  'decodeAgentProcessInfo',
  'decodeCommandAck',
  'decodeExitState',
  'enqueueProfileWork',
  'sharedServerStatus',
  'decodeAccounts',
  'decodeMaintenanceSnapshot',
  'decodeServers',
  'decodeParties',
  'decodeFederation',
  'decodeGroupDetails',
  'decodeCatalog',
  'decodeReadItem',
  'decodeCopy',
  'decodeDownload',
  'decodeMutation',
  'decodeCheckedProfile',
  'decodePendingOperations',
  'decodeBackupPhrase',
  'decodeGroupDiscovery',
  'decodeCompatibility',
  'decodeProfileReconciliation',
  'decodeServerStatus',
  'decodeAppInfo',
  'decodeGoProfileDiscovery',
  'decodeAppLockState',
  'decodeCheckedServer',
  'decodeServerLabelResponse',
  'decodeAccountDevices',
  'decodeBackupEnrollments',
  'decodeBackupRevocation',
  'decodePairingOffer',
  'decodeDeviceProvision',
  'decodeDeviceRemoval',
  'decodePassphraseReport',
  'decodePassphraseStatus',
  'decodeResetPreview',
  'decodeYubiCards',
  'decodeYubiAccounts',
  'decodeYubiResult',
  'normalizeCommandError',
  'shouldReportPassiveServerStatusError',
  'decodeAgentStatus',
  'tauriBridge',
  'isNativeHost',
  'mockRequested',
  'selectBridge',
  'discoverUnboundTeams',
  'loadSnapshot',
  'loadProfileSnapshot',
  'roleDto',
];

test('the bridge facade preserves its public type and value exports', async () => {
  const facade = new URL('../src/bridge.ts', import.meta.url);
  const statements = (await source(facade)).statements;
  assert.equal(statements.length, 1);
  const declaration = statements[0];
  assert.ok(ts.isExportDeclaration(declaration));
  assert.equal(declaration.exportClause, undefined);
  assert.ok(
    declaration.moduleSpecifier &&
      ts.isStringLiteral(declaration.moduleSpecifier),
  );
  assert.equal(declaration.moduleSpecifier.text, './bridge/index');
  const exports = await exportsOf(facade);
  assert.deepEqual(
    [...exports]
      .filter(([, kind]) => kind === 'type')
      .map(([name]) => name)
      .sort(),
    [...publicTypes].sort(),
  );
  assert.deepEqual(
    [...exports]
      .filter(([, kind]) => kind === 'value')
      .map(([name]) => name)
      .sort(),
    [...publicValues].sort(),
  );
});

test('native command maps cover the contract without duplicate methods', async () => {
  const contract = await source(
    new URL('../src/bridge/contract.ts', import.meta.url),
  );
  const bridge = contract.statements.find(
    (node): node is ts.InterfaceDeclaration =>
      ts.isInterfaceDeclaration(node) && node.name.text === 'Bridge',
  );
  assert.ok(bridge);
  const expected = bridge.members
    .filter(ts.isMethodSignature)
    .map((member) => member.name.getText(contract))
    .sort();
  const actual: string[] = [];
  for (const module of [
    'commands-accounts',
    'commands-core',
    'commands-enrollment',
    'commands-servers',
    'commands-vault',
    'commands-yubikey',
    'tauri',
  ]) {
    const file = await source(
      new URL(`../src/bridge/${module}.ts`, import.meta.url),
    );
    for (const statement of file.statements) {
      if (!ts.isVariableStatement(statement)) continue;
      for (const declaration of statement.declarationList.declarations) {
        if (
          !declaration.initializer ||
          !ts.isObjectLiteralExpression(declaration.initializer)
        )
          continue;
        for (const property of declaration.initializer.properties) {
          if (ts.isSpreadAssignment(property)) continue;
          assert.ok(ts.isPropertyAssignment(property));
          const name = property.name.getText(file);
          if (name !== 'native') actual.push(name);
        }
      }
    }
  }
  assert.equal(new Set(actual).size, actual.length);
  assert.deepEqual(actual.sort(), expected);
});
