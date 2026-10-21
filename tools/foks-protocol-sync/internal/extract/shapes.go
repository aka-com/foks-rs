package extract

import (
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"

	"github.com/aka-com/foks-rs/tools/foks-protocol-sync/internal/model"
)

const internalSuffix = "Internal__"

// Shapes records the positional layout of every generated library and remote
// struct. A generated `XInternal__` struct whose first field is the
// `codec:",toarray"` marker is what go-codec puts on the wire for X: an array
// of its remaining fields in declaration order. Unions (which carry a
// `Switch__` field) and enums have no such layout and are skipped.
func Shapes(moduleDir string, identity model.SourceIdentity) (model.ShapeArtifact, error) {
	root, err := filepath.EvalSymlinks(moduleDir)
	if err != nil {
		return model.ShapeArtifact{}, fmt.Errorf("resolve module directory: %w", err)
	}
	root, err = filepath.Abs(root)
	if err != nil {
		return model.ShapeArtifact{}, err
	}
	artifact := model.ShapeArtifact{SchemaVersion: model.ShapeSchemaVersion, Source: identity}
	seen := make(map[string]string)
	for _, directory := range []string{"proto/lib", "proto/rem"} {
		pkg := filepath.Base(directory)
		files, err := filesWithExtension(root, directory, ".go")
		if err != nil {
			return model.ShapeArtifact{}, err
		}
		for _, relative := range files {
			path, err := checkedFile(root, relative)
			if err != nil {
				return model.ShapeArtifact{}, err
			}
			parsed, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
			if err != nil {
				return model.ShapeArtifact{}, fmt.Errorf("parse %s: %w", relative, err)
			}
			shapes, err := fileShapes(parsed, pkg, relative)
			if err != nil {
				return model.ShapeArtifact{}, err
			}
			for _, shape := range shapes {
				if previous, ok := seen[shape.Name]; ok {
					return model.ShapeArtifact{}, fmt.Errorf("struct %s is defined in %s and %s", shape.Name, previous, relative)
				}
				seen[shape.Name] = relative
				artifact.Structs = append(artifact.Structs, shape)
			}
		}
	}
	artifact.Normalize()
	if err := artifact.Validate(); err != nil {
		return model.ShapeArtifact{}, err
	}
	return artifact, nil
}

func fileShapes(parsed *ast.File, pkg, relative string) ([]model.StructShape, error) {
	var result []model.StructShape
	for _, declaration := range parsed.Decls {
		general, ok := declaration.(*ast.GenDecl)
		if !ok || general.Tok != token.TYPE {
			continue
		}
		for _, spec := range general.Specs {
			typeSpec, ok := spec.(*ast.TypeSpec)
			if !ok || !strings.HasSuffix(typeSpec.Name.Name, internalSuffix) {
				continue
			}
			structType, ok := typeSpec.Type.(*ast.StructType)
			if !ok || !positional(structType) {
				continue
			}
			name := strings.TrimSuffix(typeSpec.Name.Name, internalSuffix)
			shape := model.StructShape{Name: pkg + "." + name, GoFile: relative}
			union := false
			for _, field := range structType.Fields.List[1:] {
				if len(field.Names) == 0 {
					return nil, fmt.Errorf("%s: %s has an embedded field", relative, typeSpec.Name.Name)
				}
				fieldType, err := formatExpression(field.Type)
				if err != nil {
					return nil, err
				}
				for _, fieldName := range field.Names {
					if fieldName.Name == "Switch__" {
						union = true
					}
					shape.Fields = append(shape.Fields, model.StructField{Name: fieldName.Name, Type: fieldType})
				}
			}
			if !union && len(shape.Fields) != 0 {
				result = append(result, shape)
			}
		}
	}
	return result, nil
}

// positional reports whether the struct starts with the go-codec marker
// field `_struct struct{}` tagged `codec:",toarray"`.
func positional(structType *ast.StructType) bool {
	if structType.Fields == nil || len(structType.Fields.List) == 0 {
		return false
	}
	first := structType.Fields.List[0]
	if len(first.Names) != 1 || first.Names[0].Name != "_struct" || first.Tag == nil {
		return false
	}
	tag, err := strconv.Unquote(first.Tag.Value)
	if err != nil {
		return false
	}
	options := strings.Split(reflect.StructTag(tag).Get("codec"), ",")
	for _, option := range options[1:] {
		if option == "toarray" {
			return true
		}
	}
	return false
}
