-- Pandoc filter: normalise Open Group POSIX HTML into clean GFM.
--
-- Paired with tools/strip-boilerplate.py, which removes page furniture before
-- pandoc parses. This half handles what needs the document tree.

local INFORMATIVE_START = '<!-- INFORMATIVE-START -->'
local INFORMATIVE_END = '<!-- INFORMATIVE-END -->'

local function is_tag_id(id)
  return id ~= nil and id ~= '' and id:match('^tag_[0-9_]+$') ~= nil
end

local function stringify(el)
  return pandoc.utils.stringify(el)
end

-- The section anchors are the only stable way to cite a passage of the
-- standard, so they have to survive into the Markdown. GFM has no heading
-- attributes, so emit each as a raw anchor on the line above its heading.
function Header(el)
  local tag = is_tag_id(el.identifier) and el.identifier or nil

  local content = {}
  for _, inline in ipairs(el.content) do
    local empty_anchor =
      (inline.t == 'Span' or inline.t == 'Link')
      and #inline.content == 0
      and is_tag_id(inline.identifier)
    if empty_anchor then
      tag = tag or inline.identifier
    else
      content[#content + 1] = inline
    end
  end

  -- Drop the whitespace the removed anchor left behind.
  while #content > 0 and content[1].t == 'Space' do
    table.remove(content, 1)
  end
  el.content = content

  if not tag then
    return el
  end
  el.identifier = tag
  return {
    pandoc.RawBlock('html', string.format('<a id="%s"></a>', tag)),
    el,
  }
end

-- "The following sections are informative." / "End of informative text."
-- bracket the non-normative tail of every built-in's man page. Keep them as
-- machine-greppable markers so rule extraction can stay inside normative text.
function Div(el)
  if not el.classes:includes('box') then
    return el
  end
  local text = stringify(el)
  if text:match('following sections are informative') then
    return pandoc.RawBlock('html', INFORMATIVE_START)
  end
  if text:match('End of informative text') then
    return pandoc.RawBlock('html', INFORMATIVE_END)
  end
  return el
end

-- Option-shaded content is conditional on an option (XSI, UP, OB, …) and the
-- shading images are the only thing marking its extent.
function Image(el)
  local src = el.src or ''
  if src:match('opt%-start%.gif') then
    return pandoc.Str('[Option Start]')
  end
  if src:match('opt%-end%.gif') then
    return pandoc.Str('[Option End]')
  end
  return el
end

-- The option code itself is a popup link into help/codes.html.
function Link(el)
  if (el.target or ''):match('^javascript:') then
    return el.content
  end
  return el
end

-- GFM has no definition lists, and pandoc's fallback flattens term and
-- definition into sibling paragraphs, losing the association. A bullet list
-- with a bold term keeps it.
function DefinitionList(el)
  local items = {}
  for _, entry in ipairs(el.content) do
    local term, definitions = entry[1], entry[2]
    -- Many terms are already <b>; wrapping again yields ****Note:****.
    local heading = term
    if not (#term == 1 and term[1].t == 'Strong') then
      heading = { pandoc.Strong(term) }
    end
    local blocks = { pandoc.Para(heading) }
    for _, definition in ipairs(definitions) do
      for _, block in ipairs(definition) do
        blocks[#blocks + 1] = block
      end
    end
    items[#items + 1] = blocks
  end
  return pandoc.BulletList(items)
end

-- Man-page sections wrap their bodies in <blockquote> purely for indentation.
function BlockQuote(el)
  return el.content
end

function HorizontalRule(_)
  return {}
end

-- Leftover empty anchors ("top", the per-built-in slug) once headings have
-- taken the ones that matter.
function Span(el)
  if #el.content == 0 then
    return {}
  end
  return el
end

-- Order matters: DefinitionList must rewrite before BlockQuote unwrapping
-- flattens the man sections, and the empty-Span sweep must come last or it
-- deletes the section anchors before Header can hoist them.
return {
  { Image = Image, Link = Link },
  { DefinitionList = DefinitionList },
  { Header = Header, Div = Div, BlockQuote = BlockQuote,
    HorizontalRule = HorizontalRule },
  { Span = Span },
}
