// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package docgen

import (
	"fmt"
	"go.fuchsia.dev/fuchsia/tools/cppdocgen/clangdoc"
	"io"
	"log"
	"path/filepath"
	"strings"
)

type WriteSettings struct {
	// User visible library name ("fdio")
	LibName string

	// The source root relative to the build directory.
	BuildRelSourceRoot string

	// The include directory relative to the build directory.
	BuildRelIncludeDir string

	// Browsable URL of the source code repo. File paths get appended to this to generate source
	// links.
	RepoBaseUrl string
}

// Identifies the heading level in Markdown. Level 0 is not a real heading level but is used to
// indicate something is happening outside of any heading level.
const (
	markdownHeading0 int = 0
	markdownHeading1 int = 1
	markdownHeading2 int = 2
	markdownHeading3 int = 3
	markdownHeading4 int = 4
)

// Gets a user-visible include path given the path in the build.
func (s WriteSettings) GetUserIncludePath(path string) string {
	result, err := filepath.Rel(s.BuildRelIncludeDir, path)
	if err != nil {
		log.Fatal(err)
	}
	return result
}

// Returns the given build-relative path as being relative to the source root.
func (s WriteSettings) GetSourceRootPath(path string) string {
	result, err := filepath.Rel(s.BuildRelSourceRoot, path)
	if err != nil {
		log.Fatal(err)
	}
	return result
}

func (s WriteSettings) fileSourceLink(file string) string {
	return s.RepoBaseUrl + s.GetSourceRootPath(file)
}

func (s WriteSettings) locationSourceLink(d clangdoc.Location) string {
	return fmt.Sprintf("%s#%d", s.fileSourceLink(d.Filename), d.LineNumber)
}

func writePreHeader(f io.Writer) {
	fmt.Fprintf(f, "<pre class=\"devsite-disable-click-to-copy\">\n")
}
func writePreFooter(f io.Writer) {
	fmt.Fprintf(f, "</pre>\n\n")
}

func stripPathLeftElements(path string, stripElts int) string {
	if stripElts == 0 {
		return path
	}

	norm := filepath.ToSlash(path)
	cur_slash := 0
	for i := 0; i < len(norm); i++ {
		if norm[i] == '/' {
			cur_slash++
			if cur_slash == stripElts {
				return norm[i+1:]
			}
		}
	}

	// Not enough slashes to strip, return the original.
	return norm
}

// extractCommentHeading1 separates out the first line if it starts with a single "#". If it does
// not start with a heading, the returned |heading| string will be empty. The remainder of the
// comment (or the full comment if there was no heading) is returned in |rest|.
func extractCommentHeading1(c []clangdoc.CommentInfo) (heading string, rest []clangdoc.CommentInfo) {
	if len(c) == 0 {
		// No content, fall through to returning nothing.
	} else if c[0].Kind == "TextComment" {
		// Note that the comments start with the space following the "///"
		if strings.HasPrefix(c[0].Text, " # ") {
			// Found heading.
			heading = c[0].Text[1:]
			rest = c[1:]
		} else {
			// Got to the end, there is no heading.
			rest = c
		}
	} else {
		// Need to recurse into the next level.
		innerHeading, innerRest := extractCommentHeading1(c[0].Children)
		heading = innerHeading
		rest = c
		rest[0].Children = innerRest
	}
	return
}

// writeComment formats the given comments to the output. The heading depth is the number of "#" to
// add before any headings that appear in this text. It is used to "nest" the text in a new level.
func writeComment(cs []clangdoc.CommentInfo, headingDepth int, f io.Writer) {
	for _, c := range cs {
		switch c.Kind {
		case "ParagraphComment":
			writeComment(c.Children, headingDepth, f)

			// Put a blank line after a paragraph.
			fmt.Fprintf(f, "\n")
		case "BlockCommandComment", "FullComment":
			// Just treat command comments as normal ones. The text will be in the children.
			writeComment(c.Children, headingDepth, f)
		case "TextComment":
			// Strip one leading space if there is one.
			var line string
			if len(c.Text) > 0 && c.Text[0] == ' ' {
				line = c.Text[1:]
			} else {
				line = c.Text
			}

			// If it's a markdown heading, knock it down into our hierarchy.
			if len(line) > 0 && line[0] == '#' {
				fmt.Fprintf(f, "%s", headingMarkerAtLevel(headingDepth))
			}
			fmt.Fprintf(f, "%s\n", line)
		}
	}
}

func makeIndent(length int) (out []byte) {
	out = make([]byte, length)
	for i := 0; i < length; i++ {
		out[i] = ' '
	}
	return
}

func headingMarkerAtLevel(lv int) string {
	return strings.Repeat("#", lv)
}

var htmlEscapes = map[rune]string{
	'<': "&lt;",
	'>': "&gt;",
	'&': "&amp;",
}

func escapeHtml(s string) string {
	escaped := ""
	for _, b := range s {
		if be := htmlEscapes[b]; len(be) > 0 {
			escaped += be
		} else {
			escaped += string(b)
		}
	}
	return escaped
}

var typeRenames = map[string]string{
	// Clang emits C-style "_Bool" for some reason.
	"_Bool":                  "bool",
	"std::basic_string_view": "std::string_view",
}

// Handles some name rewriting and escaping for type names. Returns the escaped string and the
// number of bytes in the unescaped version.
func getEscapedTypeName(t clangdoc.Type) (string, int) {
	tn := t.QualifiedName()

	rewritten := typeRenames[tn]
	if len(rewritten) > 0 {
		return escapeHtml(rewritten), len(rewritten)
	} else {
		return escapeHtml(tn), len(tn)
	}
}
