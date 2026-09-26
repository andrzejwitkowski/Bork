-- Turn blockquotes that start with **NOTE.**, **TIP.**, or **WARNING.**
-- into breakable tcolorbox frames.

local colors = {
  NOTE = "noteblue",
  NOTA = "noteblue",
  TIP = "tipgreen",
  ["WSKAZÓWKA"] = "tipgreen",
  WARNING = "warnorange",
  ["OSTRZEŻENIE"] = "warnorange",
}

local function kind_of(blockquote)
  local first = blockquote.content[1]
  if not first or first.t ~= "Para" then
    return nil
  end
  local inline = first.content[1]
  if not inline or inline.t ~= "Strong" then
    return nil
  end
  local label = pandoc.utils.stringify(inline)
  local known = {
    ["NOTE."] = "NOTE",
    ["NOTA."] = "NOTA",
    ["TIP."] = "WSKAZÓWKA",
    ["WSKAZÓWKA."] = "WSKAZÓWKA",
    ["WARNING."] = "OSTRZEŻENIE",
    ["OSTRZEŻENIE."] = "OSTRZEŻENIE",
  }
  if known[label] then
    return known[label]
  end
  return nil
end

function BlockQuote(el)
  local kind = kind_of(el)
  if not kind then
    return nil
  end
  local color = colors[kind]
  local open = string.format(
    "\\begin{tcolorbox}[breakable,colback=%s!8,colframe=%s,title={%s},fonttitle=\\bfseries]",
    color,
    color,
    kind
  )
  local blocks = {
    pandoc.RawBlock("latex", open),
  }
  for _, block in ipairs(el.content) do
    table.insert(blocks, block)
  end
  table.insert(blocks, pandoc.RawBlock("latex", "\\end{tcolorbox}"))
  return blocks
end
