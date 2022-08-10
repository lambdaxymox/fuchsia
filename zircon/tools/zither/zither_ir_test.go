// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package zither_test

import (
	"fmt"
	"testing"

	"github.com/google/go-cmp/cmp"
	"go.fuchsia.dev/fuchsia/tools/fidl/lib/fidlgen"
	"go.fuchsia.dev/fuchsia/tools/fidl/lib/fidlgentest"
	"go.fuchsia.dev/fuchsia/zircon/tools/zither"
)

// Permits the comparison of unexported members of fidlgen.{Library,}Name.
var cmpNameOpt = cmp.AllowUnexported(fidlgen.LibraryName{}, fidlgen.Name{})

func TestCanSummarizeLibraryName(t *testing.T) {
	name := "this.is.an.example.library"
	ir := fidlgentest.EndToEndTest{T: t}.Single(fmt.Sprintf("library %s;", name))
	sum, err := zither.NewSummary(ir, zither.SourceDeclOrder)
	if err != nil {
		t.Fatal(err)
	}
	if sum.Name.String() != name {
		t.Errorf("expected %s; got %s", name, sum.Name)
	}
}

func TestDeclOrder(t *testing.T) {
	ir := fidlgentest.EndToEndTest{T: t}.Single(`
library example;

const A int32 = 0;
const B int32 = E;
const C int32 = A;
const D int32 = 1;
const E int32 = C;
const F int32 = B;
const G int32 = 2;
`)

	{
		sum, err := zither.NewSummary(ir, zither.SourceDeclOrder)
		if err != nil {
			t.Fatal(err)
		}

		var actual []string
		for _, decl := range sum.Decls {
			actual = append(actual, decl.Name().String())
		}
		expected := []string{
			"example/A",
			"example/B",
			"example/C",
			"example/D",
			"example/E",
			"example/F",
			"example/G",
		}
		if diff := cmp.Diff(expected, actual); diff != "" {
			t.Error(diff)
		}
	}

	{
		sum, err := zither.NewSummary(ir, zither.DependencyDeclOrder)
		if err != nil {
			t.Fatal(err)
		}

		var actual []string
		for _, decl := range sum.Decls {
			actual = append(actual, decl.Name().String())
		}
		expected := []string{
			"example/A",
			"example/C",
			"example/D", // C and D have no interdependencies, and D follows C in source.
			"example/E",
			"example/B",
			"example/F",
			"example/G",
		}
		if diff := cmp.Diff(expected, actual); diff != "" {
			t.Error(diff)
		}
	}
}

func TestFloatConstantsAreDisallowed(t *testing.T) {
	decls := []string{
		"const FLOAT32 float32 = 0.0;",
		"const FLOAT64 float64 = 0.0;",
	}

	for _, decl := range decls {
		ir := fidlgentest.EndToEndTest{T: t}.Single(fmt.Sprintf(`
library example;

%s
`, decl))

		_, err := zither.NewSummary(ir, zither.SourceDeclOrder)
		if err == nil {
			t.Fatal("expected an error")
		}
		if err.Error() != "floats are unsupported" {
			t.Errorf("unexpected error: %v", err)
		}
	}
}

func TestCanSummarizeConstants(t *testing.T) {
	ir := fidlgentest.EndToEndTest{T: t}.Single(`
library example;

const BOOL bool = false;

const BINARY_UINT8 uint8 = 0b10101111;

const HEX_UINT16 uint16 = 0xabcd;

const DECIMAL_UINT32 uint32 = 123456789;

const BINARY_INT8 int8 = 0b1111010;

const HEX_INT16 int16 = 0xcba;

const NEGATIVE_HEX_INT16 int16 = -0xcba;

const DECIMAL_INT32 int32 = 1050065;

const NEGATIVE_DECIMAL_INT32 int32 = -1050065;

const UINT64_MAX uint64 = 0xffffffffffffffff;

const INT64_MIN int64 = -0x8000000000000000;

const SOME_STRING string = "XXX";

const DEFINED_IN_TERMS_OF_ANOTHER_STRING string = SOME_STRING;

const DEFINED_IN_TERMS_OF_ANOTHER_UINT16 uint16 = HEX_UINT16;

/// This is a one-line comment.
const COMMENTED_BOOL bool = true;

/// This is
///   a
///       many-line
/// comment.
const COMMENTED_STRING string = "YYY";
`)
	sum, err := zither.NewSummary(ir, zither.SourceDeclOrder)
	if err != nil {
		t.Fatal(err)
	}

	var actual []zither.Const
	for _, decl := range sum.Decls {
		if decl.IsConst() {
			actual = append(actual, decl.AsConst())
		}
	}

	someStringName := fidlgen.MustReadName("example/SOME_STRING")
	hexUint16Name := fidlgen.MustReadName("example/HEX_UINT16")

	// Listed in declaration order for readability, but similarly sorted.
	expected := []zither.Const{
		{
			Name:  fidlgen.MustReadName("example/BOOL"),
			Kind:  zither.TypeKindBool,
			Type:  "bool",
			Value: "false",
		},
		{
			Name:  fidlgen.MustReadName("example/BINARY_UINT8"),
			Kind:  zither.TypeKindInteger,
			Type:  "uint8",
			Value: "0b10101111",
		},
		{
			Name:  fidlgen.MustReadName("example/HEX_UINT16"),
			Kind:  zither.TypeKindInteger,
			Type:  "uint16",
			Value: "0xabcd",
		},
		{
			Name:  fidlgen.MustReadName("example/DECIMAL_UINT32"),
			Kind:  zither.TypeKindInteger,
			Type:  "uint32",
			Value: "123456789",
		},
		{
			Name:  fidlgen.MustReadName("example/BINARY_INT8"),
			Kind:  zither.TypeKindInteger,
			Type:  "int8",
			Value: "0b1111010",
		},
		{
			Name:  fidlgen.MustReadName("example/HEX_INT16"),
			Kind:  zither.TypeKindInteger,
			Type:  "int16",
			Value: "0xcba",
		},
		{
			Name:  fidlgen.MustReadName("example/NEGATIVE_HEX_INT16"),
			Kind:  zither.TypeKindInteger,
			Type:  "int16",
			Value: "-0xcba",
		},
		{
			Name:  fidlgen.MustReadName("example/DECIMAL_INT32"),
			Kind:  zither.TypeKindInteger,
			Type:  "int32",
			Value: "1050065",
		},
		{
			Name:  fidlgen.MustReadName("example/NEGATIVE_DECIMAL_INT32"),
			Kind:  zither.TypeKindInteger,
			Type:  "int32",
			Value: "-1050065",
		},
		{
			Name:  fidlgen.MustReadName("example/UINT64_MAX"),
			Kind:  zither.TypeKindInteger,
			Type:  "uint64",
			Value: "0xffffffffffffffff",
		},
		{
			Name:  fidlgen.MustReadName("example/INT64_MIN"),
			Kind:  zither.TypeKindInteger,
			Type:  "int64",
			Value: "-0x8000000000000000",
		},
		{
			Name:  fidlgen.MustReadName("example/SOME_STRING"),
			Kind:  zither.TypeKindString,
			Type:  "string",
			Value: "XXX",
		},
		{
			Name:       fidlgen.MustReadName("example/DEFINED_IN_TERMS_OF_ANOTHER_STRING"),
			Kind:       zither.TypeKindString,
			Type:       "string",
			Value:      "XXX",
			Identifier: &someStringName,
		},
		{
			Name:       fidlgen.MustReadName("example/DEFINED_IN_TERMS_OF_ANOTHER_UINT16"),
			Kind:       zither.TypeKindInteger,
			Type:       "uint16",
			Value:      "43981",
			Identifier: &hexUint16Name,
		},
		{
			Name:     fidlgen.MustReadName("example/COMMENTED_BOOL"),
			Kind:     zither.TypeKindBool,
			Type:     "bool",
			Value:    "true",
			Comments: []string{" This is a one-line comment."},
		},
		{
			Name:     fidlgen.MustReadName("example/COMMENTED_STRING"),
			Kind:     zither.TypeKindString,
			Type:     "string",
			Value:    "YYY",
			Comments: []string{" This is", "   a", "       many-line", " comment."},
		},
	}

	if diff := cmp.Diff(expected, actual, cmpNameOpt); diff != "" {
		t.Error(diff)
	}
}
