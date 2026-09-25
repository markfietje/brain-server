import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeAll, expect, test } from 'vitest';
import openapiTS, { astToString } from 'openapi-typescript';
import ts from 'typescript';

const OPENAPI_PATH = join(process.cwd(), '..', 'openapi.yaml');
const EXPECTED_PROPOSAL_KINDS = [
	'fact',
	'procedure',
	'step',
	'decision',
	'episodic',
	'entitlement',
	'draft',
	'channel/template',
	'registry_lifecycle'
];
const CANONICAL_ROW_PROPERTIES = [
	'id',
	'version',
	'kind',
	'name',
	'output_vocabulary',
	'artifact_digest',
	'config_digest',
	'calibration_ref',
	'status',
	'evaluation_refs',
	'proposed_by',
	'approved_by',
	'created_at',
	'updated_at'
];
const DETAIL_PROPERTY_ALLOWLIST = [...CANONICAL_ROW_PROPERTIES, 'row_digest'].sort();
const FORBIDDEN_CONTENT_PROPERTIES = [
	'rules',
	'rules_config',
	'model',
	'model_bytes',
	'weights',
	'weights_bytes',
	'query',
	'evidence',
	'evidence_text',
	'evaluation',
	'evaluation_content',
	'judgment',
	'judgment_content'
];

let generatedSourceFile: ts.SourceFile | null = null;
let rowDigestPattern: string | null = null;

function propertyNameOrNull(name: ts.PropertyName): string | null {
	if (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name)) {
		return name.text;
	}
	return null;
}

function propertyName(name: ts.PropertyName): string {
	const value = propertyNameOrNull(name);
	if (value === null) throw new Error('OpenAPI property uses an unsupported computed name');
	return value;
}

function propertySignature(
	members: readonly ts.TypeElement[],
	name: string
): ts.PropertySignature {
	const property = members.find(
		(member): member is ts.PropertySignature =>
			ts.isPropertySignature(member) && propertyName(member.name) === name
	);
	if (property === undefined) throw new Error(`OpenAPI generated property is missing: ${name}`);
	return property;
}

function typeLiteral(property: ts.PropertySignature): ts.TypeLiteralNode {
	if (property.type === undefined || !ts.isTypeLiteralNode(property.type)) {
		throw new Error(`OpenAPI property has no generated object type: ${propertyName(property.name)}`);
	}
	return property.type;
}

function nestedTypeLiteral(parent: ts.TypeLiteralNode, name: string): ts.TypeLiteralNode {
	return typeLiteral(propertySignature(parent.members, name));
}

function interfaceDeclaration(name: string): ts.InterfaceDeclaration {
	const declaration = generatedSourceFile?.statements.find(
		(statement): statement is ts.InterfaceDeclaration =>
			ts.isInterfaceDeclaration(statement) && statement.name.text === name
	);
	if (declaration === undefined) throw new Error(`Generated declaration is missing interface: ${name}`);
	return declaration;
}

function operation(name: string): ts.TypeLiteralNode {
	return typeLiteral(propertySignature(interfaceDeclaration('operations').members, name));
}

function requestContent(operationName: string): ts.TypeLiteralNode {
	const requestBody = nestedTypeLiteral(operation(operationName), 'requestBody');
	const content = nestedTypeLiteral(requestBody, 'content');
	return nestedTypeLiteral(content, 'application/json');
}

function responseContent(operationName: string, status: string): ts.TypeLiteralNode {
	const responses = nestedTypeLiteral(operation(operationName), 'responses');
	const response = nestedTypeLiteral(responses, status);
	const content = nestedTypeLiteral(response, 'content');
	return nestedTypeLiteral(content, 'application/json');
}

function unionStringLiterals(type: ts.TypeNode | undefined): string[] {
	if (type === undefined || !ts.isUnionTypeNode(type)) {
		throw new Error('OpenAPI enum did not generate a string union');
	}
	return type.types.map((member) => {
		if (!ts.isLiteralTypeNode(member) || !ts.isStringLiteral(member.literal)) {
			throw new Error('OpenAPI enum generated a non-string union member');
		}
		return member.literal.text;
	});
}

function propertyNames(type: ts.TypeLiteralNode): string[] {
	return type.members.flatMap((member) =>
		ts.isPropertySignature(member) ? [propertyName(member.name)] : []
	);
}

function requiredPropertyNames(type: ts.TypeLiteralNode): string[] {
	return type.members.flatMap((member) =>
		ts.isPropertySignature(member) && member.questionToken === undefined
			? [propertyName(member.name)]
			: []
	);
}

function rowDigestPatternValue(): string | null {
	return rowDigestPattern;
}

beforeAll(async () => {
	const source = readFileSync(OPENAPI_PATH, 'utf8');
	const ast = await openapiTS(source, {
		transformProperty(property, schemaObject) {
			const name = propertyNameOrNull(property.name);
			if (name === 'row_digest' && 'pattern' in schemaObject) {
				const pattern = schemaObject.pattern;
				if (typeof pattern === 'string') rowDigestPattern = pattern;
			}
			return property;
		}
	});
	const generated = astToString(ast);
	generatedSourceFile = ts.createSourceFile(
		'schema.d.ts',
		generated,
		ts.ScriptTarget.Latest,
		true,
		ts.ScriptKind.TS
	);
});

test('registry_lifecycle_kind_is_in_the_public_proposal_contract', () => {
	const proposal = requestContent('ingestProposal');
	const kind = propertySignature(proposal.members, 'kind');
	const actualKinds = unionStringLiterals(kind.type).sort();
	const expectedKinds = [...EXPECTED_PROPOSAL_KINDS].sort();
	expect(actualKinds).toEqual(expectedKinds);
});

test('registry_detail_omits_nothing_required_for_lifecycle_proposal', () => {
	const detail = responseContent('getModelRegistryEntry', '200');
	const properties = propertyNames(detail);
	const required = requiredPropertyNames(detail);
	const digest = propertySignature(detail.members, 'row_digest');

	expect(properties).toEqual(expect.arrayContaining(CANONICAL_ROW_PROPERTIES));
	expect(required).toContain('row_digest');
	expect(digest.type?.kind).toBe(ts.SyntaxKind.StringKeyword);
	expect(rowDigestPatternValue()).toBe('^[0-9a-f]{64}$');
});

test('registry_detail_never_exposes_model_or_evaluation_contents', () => {
	const properties = propertyNames(responseContent('getModelRegistryEntry', '200')).sort();
	expect(properties).toEqual(DETAIL_PROPERTY_ALLOWLIST);
	for (const forbidden of FORBIDDEN_CONTENT_PROPERTIES) {
		expect(properties).not.toContain(forbidden);
	}
});
