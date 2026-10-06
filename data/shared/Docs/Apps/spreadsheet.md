# Spreadsheet

Open a spreadsheet, change its cells, and save it. Formulas are worked out on the server by
[GNU Gnumeric](http://www.gnumeric.org), so the results are the same as Gnumeric's own, and the
files you save open in Excel, LibreOffice and Gnumeric.

It opens `.xlsx` (Excel), `.ods` (LibreOffice and OpenOffice), `.gnumeric` and `.csv` files,
and most other spreadsheet files Gnumeric can read.

## Finding and opening a file

The sidebar on the left lists your spreadsheets:

- **🏠 My files** is your own folder, when you are signed in.
- **👥 Shared** is everything else on the server you can see.

Click a folder (📁) to open it and see what is inside, and click it again to close it. Click a
spreadsheet (📄) to open it; the one you have open is highlighted. Only spreadsheet files are
listed, so the tree stays short.

**◀ Files** folds the sidebar away to give the grid the whole window, and **▶ Files** brings it
back.

You can also type a file's path in the **File** box, such as `/home/your-name/budget.xlsx`, and
press **📂 Open**. A big file can take a few seconds; the app says "Opening…" meanwhile.

## Starting a new spreadsheet

Press **📄 New** for an empty spreadsheet with one sheet. It isn't a file yet: to keep it, type a
name in the **File** box, such as `/home/your-name/plan.xlsx`, and press **Save as**. After that,
**💾 Save** saves to that file.

If the file has more than one sheet, each sheet's name appears as a tab above the grid. Click a
tab to show that sheet.

## Moving around

The grid shows 20 rows and 8 columns at a time, with the column letters along the top and row
numbers down the side.

- **▲** and **▼** move up and down a page of rows, and **◀** and **▶** a page of columns.
- To jump to a cell, type its name in **Go to**, such as `B20` or `AA3`, and press **Go**.

## Changing a cell

1. Click the cell. Its name appears at the left of the formula bar, with what it holds next to
   it: a value, or a formula such as `=SUM(B2:B10)`.
2. Type the new content in the formula bar. Start it with `=` for a formula. To keep something
   that looks like a number or a formula as text, start it with `'`, such as `'0042`.
3. Press **✔ Set**.

The cell shows what you typed straight away. A moment later, once the server has worked
everything out, it shows its value, and so does every cell that depends on it. "Calculating…"
shows while that happens. You can carry on changing cells meanwhile: they are worked out in turn,
in the order you made them.

To empty a cell, clear the formula bar and press **✔ Set**.

When a formula can't be worked out, the cell shows why, as spreadsheets do: `#DIV/0!` for a
division by zero, `#NAME?` for a name it doesn't know, `#REF!` for a reference to a cell that
isn't there, and so on.

Gnumeric writes function names in lower case, so `=SUM(A1:A3)` shows as `=sum(A1:A3)` once it
has been worked out. It means the same.

## Saving

- **💾 Save** saves to the file you opened, in its own format.
- **Save as** saves to the file named in the **File** box. The ending of the name chooses the
  format: `.xlsx`, `.ods`, `.gnumeric` or `.csv`. From then on, **💾 Save** saves there.

Saving as `.csv` keeps only the first sheet and only values, not formulas, because that is all a
CSV file can hold. Use one of the other formats to keep everything.

You can open and save files only where you are allowed to. Read about
[who can see what](docs.md) in the Docs page. Your own folder, `/home/your-name`, always works.
Changes are kept on the server only until you save them, so save before you close the window.

## What it doesn't do

The Spreadsheet shows values and formulas. It doesn't show charts, colours or fonts, and you
can't change them here, but it doesn't remove them either: saving keeps whatever Gnumeric keeps
when it converts the file, which for most files is all of it. Two people
can't edit the same sheet at once; each gets their own copy until they save.
