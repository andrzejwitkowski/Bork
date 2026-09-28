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

local function drop_label(para)
  local inl = para.content
  if inl[1] and inl[1].t == "Strong" then
    table.remove(inl, 1)
    if inl[1] and (inl[1].t == "Space" or inl[1].t == "SoftBreak") then
      table.remove(inl, 1)
    end
  end
  return #inl > 0
end

-- [H] jest ustawione w header.tex. Kara broni przed łamaniem strony
-- między rysunkiem a akapitem, który się do niego odwołuje.
function Figure(el)
  if FORMAT ~= "latex" and FORMAT ~= "beamer" then
    return nil
  end
  return {
    el,
    pandoc.RawBlock("latex", "\\nopagebreak"),
  }
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
  for i, block in ipairs(el.content) do
    if i == 1 and block.t == "Para" then
      if drop_label(block) then
        table.insert(blocks, block)
      end
    else
      table.insert(blocks, block)
    end
  end
  table.insert(blocks, pandoc.RawBlock("latex", "\\end{tcolorbox}"))
  return blocks
end

local function latex_escape(s)
  s = s:gsub("\\", "\\textbackslash{}")
  s = s:gsub("%%", "\\%%")
  s = s:gsub("%#", "\\#")
  s = s:gsub("%$", "\\$")
  s = s:gsub("&", "\\&")
  s = s:gsub("_", "\\_")
  s = s:gsub("{", "\\{")
  s = s:gsub("}", "\\}")
  s = s:gsub("%^", "\\textasciicircum{}")
  s = s:gsub("~", "\\textasciitilde{}")
  return s
end

function Code(el)
  if FORMAT ~= "latex" and FORMAT ~= "beamer" then
    return nil
  end
  local parts = {}
  local buf = {}
  local function flush()
    if #buf > 0 then
      table.insert(parts, latex_escape(table.concat(buf)))
      buf = {}
    end
  end
  for i = 1, #el.text do
    local c = el.text:sub(i, i)
    table.insert(buf, c)
    if c == "_" or c == "/" or c == "." or c == ":" or c == "-" then
      flush()
      table.insert(parts, "\\hspace{0pt}")
    end
  end
  flush()
  return pandoc.RawInline("latex", "\\texttt{" .. table.concat(parts) .. "}")
end
