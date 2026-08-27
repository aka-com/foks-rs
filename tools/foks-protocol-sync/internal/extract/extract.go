package extract

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"

	"github.com/aka-proj/foks-protocol-sync/internal/model"
)

type protocolVariable struct {
	id     uint32
	goFile string
}

func Module(moduleDir string, identity model.SourceIdentity) (model.Artifact, error) {
	root, err := filepath.EvalSymlinks(moduleDir)
	if err != nil {
		return model.Artifact{}, fmt.Errorf("resolve module directory: %w", err)
	}
	root, err = filepath.Abs(root)
	if err != nil {
		return model.Artifact{}, err
	}

	variables := make(map[string]protocolVariable)
	methods := make(map[string][]model.Method)
	handlers := make(map[string][]model.Method)
	protocolNames := make(map[string]string)
	remoteGoFiles, err := filesWithExtension(root, "proto/rem", ".go")
	if err != nil {
		return model.Artifact{}, err
	}
	for _, relative := range remoteGoFiles {
		path, err := checkedFile(root, relative)
		if err != nil {
			return model.Artifact{}, err
		}
		parsed, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
		if err != nil {
			return model.Artifact{}, fmt.Errorf("parse %s: %w", relative, err)
		}
		constants := fileConstants(parsed)
		for _, declaration := range parsed.Decls {
			general, ok := declaration.(*ast.GenDecl)
			if !ok || general.Tok != token.VAR {
				continue
			}
			for _, spec := range general.Specs {
				valueSpec, ok := spec.(*ast.ValueSpec)
				if !ok {
					continue
				}
				for i, name := range valueSpec.Names {
					if !strings.HasSuffix(name.Name, "ProtocolID") || i >= len(valueSpec.Values) {
						continue
					}
					value, err := integer(valueSpec.Values[i], constants)
					if err != nil || value > uint64(^uint32(0)) {
						return model.Artifact{}, fmt.Errorf("%s: invalid %s: %v", relative, name.Name, err)
					}
					if _, exists := variables[name.Name]; exists {
						return model.Artifact{}, fmt.Errorf("duplicate protocol variable %s", name.Name)
					}
					variables[name.Name] = protocolVariable{id: uint32(value), goFile: relative}
				}
			}
		}

		ast.Inspect(parsed, func(node ast.Node) bool {
			call, ok := node.(*ast.CallExpr)
			if !ok || !isSelector(call.Fun, "NewMethodV2") || len(call.Args) < 3 {
				return true
			}
			identifier, ok := call.Args[0].(*ast.Ident)
			if !ok {
				err = fmt.Errorf("%s: NewMethodV2 protocol is not an identifier", relative)
				return false
			}
			position, parseErr := integer(call.Args[1], constants)
			if parseErr != nil || position > uint64(^uint32(0)) {
				err = fmt.Errorf("%s: invalid method position: %v", relative, parseErr)
				return false
			}
			qualified, parseErr := stringLiteral(call.Args[2])
			if parseErr != nil {
				err = fmt.Errorf("%s: invalid method name: %v", relative, parseErr)
				return false
			}
			parts := strings.SplitN(qualified, ".", 2)
			if len(parts) != 2 || parts[0] == "" || parts[1] == "" {
				err = fmt.Errorf("%s: malformed qualified method %q", relative, qualified)
				return false
			}
			if old, exists := protocolNames[identifier.Name]; exists && old != parts[0] {
				err = fmt.Errorf("%s: %s names both %s and %s", relative, identifier.Name, old, parts[0])
				return false
			}
			protocolNames[identifier.Name] = parts[0]
			methods[identifier.Name] = append(methods[identifier.Name], model.Method{
				Name: parts[1], Position: uint32(position), QualifiedName: qualified,
			})
			return true
		})
		if err != nil {
			return model.Artifact{}, err
		}
		fileHandlers, err := handlerMethods(parsed, relative, constants)
		if err != nil {
			return model.Artifact{}, err
		}
		for variable, values := range fileHandlers {
			if _, exists := handlers[variable]; exists {
				return model.Artifact{}, fmt.Errorf("duplicate handler protocol %s", variable)
			}
			handlers[variable] = values
		}
	}

	artifact := model.Artifact{SchemaVersion: model.SchemaVersion, Source: identity}
	for variable, values := range methods {
		protocol, ok := variables[variable]
		if !ok {
			return model.Artifact{}, fmt.Errorf("methods reference unknown protocol variable %s", variable)
		}
		if err := compareHandlerMethods(variable, values, handlers[variable]); err != nil {
			return model.Artifact{}, err
		}
		artifact.Protocols = append(artifact.Protocols, model.Protocol{
			Name: protocolNames[variable], UniqueID: protocol.id, GoFile: protocol.goFile, Methods: values,
		})
	}
	for variable := range handlers {
		if _, ok := methods[variable]; !ok {
			return model.Artifact{}, fmt.Errorf("handler protocol %s has no client methods", variable)
		}
	}

	artifact.Statuses, err = namedConstants(root, "proto/lib/status.go", "StatusCode_")
	if err != nil {
		return model.Artifact{}, err
	}
	artifact.Services, err = namedConstants(root, "proto/lib/common.go", "ServerType_")
	if err != nil {
		return model.Artifact{}, err
	}
	sourceFiles, err := snowpackSourceFiles(root)
	if err != nil {
		return model.Artifact{}, err
	}
	for _, relative := range sourceFiles {
		path, err := checkedFile(root, relative)
		if err != nil {
			return model.Artifact{}, err
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return model.Artifact{}, err
		}
		digest := sha256.Sum256(data)
		semanticDigest := sha256.Sum256(semanticSnowpack(data))
		artifact.Sources = append(artifact.Sources, model.SourceFile{
			Path: relative, SHA256: hex.EncodeToString(digest[:]), SemanticSHA256: hex.EncodeToString(semanticDigest[:]),
		})
	}
	artifact.Normalize()
	if err := artifact.Validate(); err != nil {
		return model.Artifact{}, err
	}
	return artifact, nil
}

func semanticSnowpack(input []byte) []byte {
	output := make([]byte, 0, len(input))
	emit := func(token []byte) {
		output = strconv.AppendInt(output, int64(len(token)), 10)
		output = append(output, ':')
		output = append(output, token...)
	}
	for index := 0; index < len(input); {
		current := input[index]
		if current == '"' || current == '\'' {
			start, quote, escaped := index, current, false
			index++
			for index < len(input) {
				character := input[index]
				index++
				if escaped {
					escaped = false
				} else if character == '\\' {
					escaped = true
				} else if character == quote {
					break
				}
			}
			emit(input[start:index])
			continue
		}
		if current == '/' && index+1 < len(input) && input[index+1] == '/' {
			index += 2
			for index < len(input) && input[index] != '\n' {
				index++
			}
			continue
		}
		if current == '/' && index+1 < len(input) && input[index+1] == '*' {
			index += 2
			for index+1 < len(input) && (input[index] != '*' || input[index+1] != '/') {
				index++
			}
			if index+1 < len(input) {
				index += 2
			}
			continue
		}
		if current == ' ' || current == '\t' || current == '\r' || current == '\n' {
			index++
			continue
		}
		start := index
		if wordByte(current) {
			for index < len(input) && wordByte(input[index]) {
				index++
			}
		} else {
			index++
		}
		emit(input[start:index])
	}
	return output
}

func wordByte(value byte) bool {
	return value >= 'a' && value <= 'z' || value >= 'A' && value <= 'Z' ||
		value >= '0' && value <= '9' || value == '_'
}

func handlerMethods(parsed *ast.File, relative string, constants map[string]uint64) (map[string][]model.Method, error) {
	result := make(map[string][]model.Method)
	var extractionError error
	ast.Inspect(parsed, func(node ast.Node) bool {
		literal, ok := node.(*ast.CompositeLit)
		if !ok || !isSelector(literal.Type, "ProtocolV2") {
			return true
		}
		var protocolName, variable string
		var methodMap *ast.CompositeLit
		for _, element := range literal.Elts {
			field, ok := element.(*ast.KeyValueExpr)
			if !ok {
				continue
			}
			key, ok := field.Key.(*ast.Ident)
			if !ok {
				continue
			}
			switch key.Name {
			case "Name":
				protocolName, extractionError = stringLiteral(field.Value)
			case "ID":
				identifier, ok := field.Value.(*ast.Ident)
				if !ok {
					extractionError = fmt.Errorf("%s: ProtocolV2 ID is not an identifier", relative)
				} else {
					variable = identifier.Name
				}
			case "Methods":
				methodMap, ok = field.Value.(*ast.CompositeLit)
				if !ok {
					extractionError = fmt.Errorf("%s: ProtocolV2 Methods is not a map literal", relative)
				}
			}
			if extractionError != nil {
				return false
			}
		}
		if protocolName == "" || variable == "" || methodMap == nil {
			extractionError = fmt.Errorf("%s: incomplete ProtocolV2 literal", relative)
			return false
		}
		var values []model.Method
		for _, element := range methodMap.Elts {
			entry, ok := element.(*ast.KeyValueExpr)
			if !ok {
				extractionError = fmt.Errorf("%s: non-keyed ProtocolV2 method", relative)
				return false
			}
			position, err := integer(entry.Key, constants)
			if err != nil || position > uint64(^uint32(0)) {
				extractionError = fmt.Errorf("%s: invalid handler position: %v", relative, err)
				return false
			}
			description, ok := entry.Value.(*ast.CompositeLit)
			if !ok {
				extractionError = fmt.Errorf("%s: handler description is not a literal", relative)
				return false
			}
			methodName := ""
			for _, descriptionElement := range description.Elts {
				field, ok := descriptionElement.(*ast.KeyValueExpr)
				if !ok {
					continue
				}
				key, ok := field.Key.(*ast.Ident)
				if ok && key.Name == "Name" {
					methodName, extractionError = stringLiteral(field.Value)
					break
				}
			}
			if extractionError != nil || methodName == "" {
				if extractionError == nil {
					extractionError = fmt.Errorf("%s: handler at %d has no name", relative, position)
				}
				return false
			}
			values = append(values, model.Method{
				Name: methodName, Position: uint32(position), QualifiedName: protocolName + "." + methodName,
			})
		}
		result[variable] = values
		return false
	})
	return result, extractionError
}

func compareHandlerMethods(variable string, client, handler []model.Method) error {
	if len(client) != len(handler) {
		return fmt.Errorf("%s client/handler method count differs: %d != %d", variable, len(client), len(handler))
	}
	byPosition := make(map[uint32]model.Method)
	for _, method := range handler {
		byPosition[method.Position] = method
	}
	for _, method := range client {
		other, ok := byPosition[method.Position]
		if !ok || other.Name != method.Name || other.QualifiedName != method.QualifiedName {
			return fmt.Errorf("%s client/handler mismatch at position %d", variable, method.Position)
		}
	}
	return nil
}

func snowpackSourceFiles(root string) ([]string, error) {
	result, err := filesWithExtension(root, "proto-src/lib", ".snowp")
	if err != nil {
		return nil, err
	}
	remote, err := filesWithExtension(root, "proto-src/rem", ".snowp")
	if err != nil {
		return nil, err
	}
	result = append(result, remote...)
	sort.Strings(result)
	return result, nil
}

func filesWithExtension(root, relativeDirectory, extension string) ([]string, error) {
	directory, err := checkedDirectory(root, relativeDirectory)
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(directory)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", relativeDirectory, err)
	}
	var result []string
	for _, entry := range entries {
		if filepath.Ext(entry.Name()) != extension || strings.HasSuffix(entry.Name(), "_test.go") {
			continue
		}
		result = append(result, filepath.ToSlash(filepath.Join(relativeDirectory, entry.Name())))
	}
	if len(result) == 0 {
		return nil, fmt.Errorf("%s contains no %s files", relativeDirectory, extension)
	}
	sort.Strings(result)
	return result, nil
}

func namedConstants(root, relative, prefix string) ([]model.NamedValue, error) {
	path, err := checkedFile(root, relative)
	if err != nil {
		return nil, err
	}
	parsed, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
	if err != nil {
		return nil, fmt.Errorf("parse %s: %w", relative, err)
	}
	var result []model.NamedValue
	known := make(map[string]uint64)
	for _, declaration := range parsed.Decls {
		general, ok := declaration.(*ast.GenDecl)
		if !ok || general.Tok != token.CONST {
			continue
		}
		for _, spec := range general.Specs {
			valueSpec, ok := spec.(*ast.ValueSpec)
			if !ok {
				continue
			}
			for i, name := range valueSpec.Names {
				if i >= len(valueSpec.Values) {
					continue
				}
				value, err := integer(valueSpec.Values[i], known)
				if err != nil || value > uint64(^uint32(0)) {
					return nil, fmt.Errorf("%s: invalid %s: %v", relative, name.Name, err)
				}
				known[name.Name] = value
				if !strings.HasPrefix(name.Name, prefix) {
					continue
				}
				result = append(result, model.NamedValue{
					Name: strings.TrimPrefix(name.Name, prefix), Value: int64(value), GoFile: relative,
				})
			}
		}
	}
	if len(result) == 0 {
		return nil, fmt.Errorf("%s: found no %s constants", relative, prefix)
	}
	return result, nil
}

func checkedFile(root, relative string) (string, error) {
	if filepath.IsAbs(relative) || strings.Contains(relative, "..") {
		return "", fmt.Errorf("unsafe source path %q", relative)
	}
	path := filepath.Join(root, filepath.FromSlash(relative))
	info, err := os.Lstat(path)
	if err != nil {
		return "", fmt.Errorf("inspect %s: %w", relative, err)
	}
	if info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
		return "", fmt.Errorf("%s is not a regular non-symlink file", relative)
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	prefix := root + string(os.PathSeparator)
	if !strings.HasPrefix(abs, prefix) {
		return "", fmt.Errorf("%s escapes module root", relative)
	}
	resolved, err := filepath.EvalSymlinks(abs)
	if err != nil {
		return "", fmt.Errorf("resolve %s: %w", relative, err)
	}
	if resolved != abs {
		return "", fmt.Errorf("%s traverses a symlink", relative)
	}
	return abs, nil
}

func checkedDirectory(root, relative string) (string, error) {
	if filepath.IsAbs(relative) || strings.Contains(relative, "..") {
		return "", fmt.Errorf("unsafe source directory %q", relative)
	}
	path := filepath.Join(root, filepath.FromSlash(relative))
	info, err := os.Lstat(path)
	if err != nil {
		return "", fmt.Errorf("inspect %s: %w", relative, err)
	}
	if info.Mode()&os.ModeSymlink != 0 || !info.IsDir() {
		return "", fmt.Errorf("%s is not a non-symlink directory", relative)
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	resolved, err := filepath.EvalSymlinks(abs)
	if err != nil {
		return "", fmt.Errorf("resolve %s: %w", relative, err)
	}
	if resolved != abs || !strings.HasPrefix(abs, root+string(os.PathSeparator)) {
		return "", fmt.Errorf("%s escapes the module root or traverses a symlink", relative)
	}
	return abs, nil
}

func fileConstants(parsed *ast.File) map[string]uint64 {
	known := make(map[string]uint64)
	for _, declaration := range parsed.Decls {
		general, ok := declaration.(*ast.GenDecl)
		if !ok || general.Tok != token.CONST {
			continue
		}
		for _, spec := range general.Specs {
			valueSpec, ok := spec.(*ast.ValueSpec)
			if !ok {
				continue
			}
			for i, name := range valueSpec.Names {
				if i >= len(valueSpec.Values) {
					continue
				}
				if value, err := integer(valueSpec.Values[i], known); err == nil {
					known[name.Name] = value
				}
			}
		}
	}
	return known
}

func integer(expression ast.Expr, names map[string]uint64) (uint64, error) {
	switch value := expression.(type) {
	case *ast.BasicLit:
		if value.Kind != token.INT {
			return 0, fmt.Errorf("expected integer literal")
		}
		return strconv.ParseUint(value.Value, 0, 64)
	case *ast.CallExpr:
		if len(value.Args) != 1 {
			return 0, fmt.Errorf("integer conversion has %d arguments", len(value.Args))
		}
		return integer(value.Args[0], names)
	case *ast.ParenExpr:
		return integer(value.X, names)
	case *ast.UnaryExpr:
		if value.Op != token.ADD {
			return 0, fmt.Errorf("unsupported unary operator %s", value.Op)
		}
		return integer(value.X, names)
	case *ast.Ident:
		resolved, ok := names[value.Name]
		if !ok {
			return 0, fmt.Errorf("unknown integer alias %s", value.Name)
		}
		return resolved, nil
	default:
		return 0, fmt.Errorf("unsupported integer expression %T", expression)
	}
}

func stringLiteral(expression ast.Expr) (string, error) {
	literal, ok := expression.(*ast.BasicLit)
	if !ok || literal.Kind != token.STRING {
		return "", fmt.Errorf("expected string literal")
	}
	return strconv.Unquote(literal.Value)
}

func isSelector(expression ast.Expr, name string) bool {
	selector, ok := expression.(*ast.SelectorExpr)
	return ok && selector.Sel.Name == name
}
