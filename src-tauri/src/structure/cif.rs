//! mmCIF reader.
//!
//! mmCIF is the format that matters going forward: it is what the PDB serves, what
//! AlphaFold 3 and Boltz write, and the only one of the two that can represent a
//! structure with more than 62 chains or 99,999 atoms. So this is a real tokeniser for
//! the subset that describes coordinates, not a line-oriented guess.
//!
//! Handled: `data_` blocks, key-value items, `loop_` tables, single and double quoted
//! values, and semicolon-delimited multi-line text fields. Not handled: save frames and
//! `stop_`, neither of which appears in coordinate files.

use super::bonds::{AtomRef, Connection, Evidence};
use super::chem;
use super::components::Template;
use super::model::{BondKind, BondOrder, Linking, RawAtom, SecondaryStructure, Structure};
use std::collections::HashMap;

/// A value in a CIF file. `.` and `?` are the format's "not applicable" and "unknown",
/// which must be distinguished from a literal string so a missing B-factor does not
/// become 0.0 by accident.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Text(String),
    Absent,
}

impl Value {
    fn as_str(&self) -> &str {
        match self {
            Value::Text(text) => text,
            Value::Absent => "",
        }
    }

    /// A finite number. Rust's parser accepts `nan` and `inf`, which are no more a
    /// coordinate or a B-factor than `?` is.
    fn parse_f32(&self) -> Option<f32> {
        match self {
            Value::Text(text) => text.parse::<f32>().ok().filter(|value| value.is_finite()),
            Value::Absent => None,
        }
    }

    fn parse_i32(&self) -> Option<i32> {
        match self {
            Value::Text(text) => text.parse().ok(),
            Value::Absent => None,
        }
    }
}

struct Tokenizer<'a> {
    bytes: &'a [u8],
    position: usize,
}

#[derive(Debug, PartialEq)]
enum Token {
    /// A bare, quoted, or multi-line value.
    Value(Value),
    /// `_category.item`
    Tag(String),
    Loop,
    /// `data_xxx` or `save_`
    Block(String),
    End,
}

impl<'a> Tokenizer<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            position: 0,
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while self.position < self.bytes.len()
                && self.bytes[self.position].is_ascii_whitespace()
            {
                self.position += 1;
            }
            // A '#' starts a comment only at the beginning of a token, never inside a
            // value - which is why comment skipping lives here and not in the reader.
            if self.position < self.bytes.len() && self.bytes[self.position] == b'#' {
                while self.position < self.bytes.len() && self.bytes[self.position] != b'\n' {
                    self.position += 1;
                }
                continue;
            }
            return;
        }
    }

    fn at_line_start(&self) -> bool {
        self.position == 0 || self.bytes[self.position - 1] == b'\n'
    }

    fn next(&mut self) -> Token {
        self.skip_whitespace_and_comments();
        if self.position >= self.bytes.len() {
            return Token::End;
        }

        let start_of_line = self.at_line_start();
        let first = self.bytes[self.position];

        // Semicolon text field: only when the semicolon is the first character of a line.
        if first == b';' && start_of_line {
            self.position += 1;
            let start = self.position;
            let mut end = self.bytes.len();
            while self.position < self.bytes.len() {
                if self.bytes[self.position] == b'\n'
                    && self.position + 1 < self.bytes.len()
                    && self.bytes[self.position + 1] == b';'
                {
                    end = self.position;
                    self.position += 2;
                    break;
                }
                self.position += 1;
            }
            // Line breaks inside the field are the file's, and a file saved on Windows
            // would otherwise carry a CR into every title shown from it.
            let text = String::from_utf8_lossy(&self.bytes[start..end])
                .trim()
                .replace("\r\n", "\n");
            return Token::Value(Value::Text(text));
        }

        if first == b'\'' || first == b'"' {
            let quote = first;
            self.position += 1;
            let start = self.position;
            // A quote closes the value only when followed by whitespace or end of input;
            // apostrophes inside names like 5'-end are common and must not terminate it.
            while self.position < self.bytes.len() {
                if self.bytes[self.position] == quote
                    && (self.position + 1 >= self.bytes.len()
                        || self.bytes[self.position + 1].is_ascii_whitespace())
                {
                    break;
                }
                self.position += 1;
            }
            let text = String::from_utf8_lossy(&self.bytes[start..self.position]).to_string();
            self.position = (self.position + 1).min(self.bytes.len());
            return Token::Value(Value::Text(text));
        }

        let start = self.position;
        while self.position < self.bytes.len() && !self.bytes[self.position].is_ascii_whitespace() {
            self.position += 1;
        }
        let word = String::from_utf8_lossy(&self.bytes[start..self.position]).to_string();

        if word.eq_ignore_ascii_case("loop_") {
            return Token::Loop;
        }
        if let Some(rest) = word.strip_prefix("data_") {
            return Token::Block(rest.to_string());
        }
        if word.starts_with('_') {
            return Token::Tag(word.to_ascii_lowercase());
        }
        if word == "." || word == "?" {
            return Token::Value(Value::Absent);
        }
        Token::Value(Value::Text(word))
    }
}

/// A parsed `loop_` table: column tags in order, and rows of values.
struct Loop {
    columns: HashMap<String, usize>,
    rows: Vec<Vec<Value>>,
}

impl Loop {
    fn get(&self, row: &[Value], tag: &str) -> Value {
        match self.columns.get(tag) {
            Some(&index) => row.get(index).cloned().unwrap_or(Value::Absent),
            None => Value::Absent,
        }
    }

    fn has(&self, tag: &str) -> bool {
        self.columns.contains_key(tag)
    }
}

/// Parses a file, and reports what it said about bonding.
///
/// The two travel together because they come from the same file: `_chem_comp_bond` is the
/// dictionary for exactly the components in this entry - its ligands included - and
/// `_struct_conn` is the list of joins no dictionary can hold, so a reader that returned
/// only coordinates would leave the bond graph to be guessed.
pub fn parse(text: &str, source: &str) -> (Structure, Evidence) {
    parse_unit(text, source, Unit::Deposited)
}

/// Which of the things an entry describes is built from it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Unit {
    /// The coordinates as they are written: the asymmetric unit of a crystal entry.
    #[default]
    Deposited,
    /// The entry's first biological assembly, made from the deposited chains by the
    /// operators the file lists for it.
    Assembly,
    /// One assembly by its id in `_pdbx_struct_assembly` - a capsid's pentamer, a crystal's
    /// own unit cell contents - or the first, if the file has no such id.
    AssemblyId(String),
}

/// How an entry is read: which assembly, which model, which alternate locations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Reading {
    pub unit: Unit,
    /// A model number; the first model, unsaid.
    pub model: Option<String>,
    /// An alternate-location label to keep where a residue has it; otherwise each
    /// residue's most occupied conformer.
    pub alternate: Option<char>,
}

impl From<Unit> for Reading {
    fn from(unit: Unit) -> Self {
        Reading {
            unit,
            ..Reading::default()
        }
    }
}

/// Reads an mmCIF file, as deposited or as its first biological assembly.
pub fn parse_unit(text: &str, source: &str, unit: Unit) -> (Structure, Evidence) {
    parse_reading(text, source, &Reading::from(unit))
}

/// Reads an mmCIF file the way `reading` says.
pub fn parse_reading(text: &str, source: &str, reading: &Reading) -> (Structure, Evidence) {
    let unit = &reading.unit;
    let mut tokenizer = Tokenizer::new(text);
    let mut atom_site: Option<Loop> = None;
    let mut struct_conf: Option<Loop> = None;
    let mut sheet_range: Option<Loop> = None;
    let mut chem_comp: Option<Loop> = None;
    let mut chem_comp_bond: Option<Loop> = None;
    let mut struct_conn: Option<Loop> = None;
    let mut struct_asym: Option<Loop> = None;
    let mut entity: Option<Loop> = None;
    let mut assembly_gen: Option<Loop> = None;
    let mut oper_list: Option<Loop> = None;
    let mut assemblies: Option<Loop> = None;
    let mut title = String::new();
    let mut entry_id = String::new();
    let mut method: Option<String> = None;

    // Key-value items outside loops, e.g. `_struct.title  'Lysozyme'`.
    let mut pending_tag: Option<String> = None;
    // A category with a single row is written as key-value lines rather than as a
    // `loop_` - the same table, one row long. An entry with exactly one disulfide, one
    // helix or one chain has its `_struct_conn`, `_struct_conf` or `_struct_asym` in this
    // form, and a reader that only looks inside loops loses it. So the pairs of each
    // category are gathered into a one-row table and filed like any other.
    let mut single: Option<(String, Loop)> = None;
    // Whether a block has given up its atoms yet. Tracked beside `file` because the
    // tables it fills are borrowed by it until the read is done.
    let mut coordinates_read = false;

    let mut file = |parsed: Loop| {
        if parsed.has("_exptl.method") {
            method = parsed
                .rows
                .first()
                .map(|row| parsed.get(row, "_exptl.method").as_str().trim().to_string())
                .filter(|method| !method.is_empty());
        } else if parsed.has("_atom_site.cartn_x") {
            atom_site = Some(parsed);
        } else if parsed.has("_struct_conf.conf_type_id") {
            struct_conf = Some(parsed);
        } else if parsed.has("_chem_comp_bond.atom_id_1") {
            chem_comp_bond = Some(parsed);
        } else if parsed.has("_chem_comp.id") && parsed.has("_chem_comp.type") {
            chem_comp = Some(parsed);
        } else if parsed.has("_struct_conn.conn_type_id") {
            struct_conn = Some(parsed);
        } else if parsed.has("_struct_asym.entity_id") {
            struct_asym = Some(parsed);
        } else if parsed.has("_pdbx_struct_assembly_gen.oper_expression") {
            assembly_gen = Some(parsed);
        } else if parsed.has("_pdbx_struct_oper_list.id") {
            oper_list = Some(parsed);
        } else if parsed.has("_pdbx_struct_assembly.id") {
            assemblies = Some(parsed);
        } else if parsed.has("_entity.type") && parsed.has("_entity.id") {
            entity = Some(parsed);
        } else if parsed.has("_struct_sheet_range.beg_auth_seq_id")
            || parsed.has("_struct_sheet_range.beg_label_seq_id")
        {
            // Beta strands live in their own category, not in _struct_conf.
            // Reading only _struct_conf finds zero sheets in every file the PDB
            // serves - a beta-grasp fold like ubiquitin comes out entirely coil.
            sheet_range = Some(parsed);
        }
    };

    loop {
        let token = tokenizer.next();
        // Anything but another item of the same category closes the one-row table.
        let continues = match (&token, &single, &pending_tag) {
            (Token::Tag(tag), Some((category, _)), _) => category_of(tag) == category,
            (Token::Value(_), Some(_), Some(_)) => true,
            _ => false,
        };
        if !continues {
            if let Some((_, table)) = single.take() {
                coordinates_read |= table.has("_atom_site.cartn_x");
                file(table);
            }
        }
        match token {
            Token::End => break,
            Token::Block(id) => {
                // One structure per file: the first block that has coordinates. Reading on
                // took the atoms of the last block and the title, chains and secondary
                // structure of whichever block happened to carry them - two structures
                // spliced into one. A leading block without coordinates (a component
                // dictionary written ahead of the model) is read on past, and the block
                // that does have them names the entry.
                if coordinates_read {
                    break;
                }
                entry_id = id;
                pending_tag = None;
            }
            Token::Tag(tag) => {
                pending_tag = Some(tag);
            }
            Token::Value(value) => {
                if let Some(tag) = pending_tag.take() {
                    if tag == "_struct.title" || tag == "_entry.title" {
                        title = value.as_str().to_string();
                    }
                    let (_, table) = single.get_or_insert_with(|| {
                        (
                            category_of(&tag).to_string(),
                            Loop {
                                columns: HashMap::new(),
                                rows: vec![Vec::new()],
                            },
                        )
                    });
                    if !table.columns.contains_key(&tag) {
                        table.columns.insert(tag, table.rows[0].len());
                        table.rows[0].push(value);
                    }
                }
            }
            Token::Loop => {
                let parsed = read_loop(&mut tokenizer);
                pending_tag = None;
                coordinates_read |= parsed.has("_atom_site.cartn_x");
                file(parsed);
            }
        }
    }
    let title = if title.is_empty() {
        entry_id.clone()
    } else {
        title
    };
    let read = match &atom_site {
        Some(table) => read_atoms(table, reading.model.as_deref(), reading.alternate),
        None => ReadAtoms::default(),
    };
    let listed = list_assemblies(assemblies.as_ref(), assembly_gen.as_ref());
    let wanted = match unit {
        Unit::Deposited => None,
        Unit::Assembly => listed.first().map(|assembly| assembly.id.clone()),
        Unit::AssemblyId(id) => listed
            .iter()
            .find(|assembly| assembly.id == *id)
            .or(listed.first())
            .map(|assembly| assembly.id.clone()),
    };
    let (atoms, built) = match &wanted {
        None => (read.atoms, None),
        Some(id) => build_assembly(read.atoms, assembly_gen.as_ref(), oper_list.as_ref(), id),
    };

    let mut structure = Structure::build(title, source.to_string(), atoms);
    structure.entry_id = entry_id;
    structure.versions = super::model::Versions {
        assemblies: listed,
        models: read.models,
        alternates: read.alternates.iter().map(char::to_string).collect(),
        alternate_residues: read.doubled,
        assembly: built,
        model: read.model,
        alternate: reading.alternate.map(|label| label.to_string()),
    };
    structure.experimental_method = method;
    structure.assign_entities(&read_entities(struct_asym.as_ref(), entity.as_ref()));
    if let Some(table) = struct_conf {
        apply_struct_conf(&mut structure, &table);
    }
    if let Some(table) = sheet_range {
        apply_sheet_ranges(&mut structure, &table);
    }

    let evidence = Evidence {
        templates: read_templates(chem_comp_bond.as_ref(), chem_comp.as_ref()),
        connections: struct_conn
            .as_ref()
            .map(read_connections)
            .unwrap_or_default(),
    };
    (structure, evidence)
}

/// The operator ids an `oper_expression` names, in order.
///
/// After StructCooker's `parse_expression` (`instructions/transforms/cif.py`), whose cases
/// these are: `1` is one operator; `(1,2,6)` and `P,Q` are lists; `(1-60)` and `1-10,21-25`
/// are ranges, expanded when both ends are numbers. A product of two groups, `(X0)(1-60)`,
/// is `None`: it means every operator of the first applied after every one of the second,
/// which StructCooker leaves out, and so does this - the row is skipped.
pub fn operators_of(expression: &str) -> Option<Vec<String>> {
    if expression.contains(")(") {
        return None;
    }
    // A list and/or ranges, without parentheses.
    fn expand(text: &str) -> Vec<String> {
        let range = |item: &str| -> Option<Vec<String>> {
            let (start, end) = item.split_once('-')?;
            let (start, end) = (start.trim(), end.trim());
            let numeric = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
            if !numeric(start) || !numeric(end) {
                return None;
            }
            let (start, end) = (start.parse::<u64>().ok()?, end.parse::<u64>().ok()?);
            // A range written backwards, or absurdly long, names nothing.
            if end < start || end - start > 100_000 {
                return Some(Vec::new());
            }
            Some((start..=end).map(|value| value.to_string()).collect())
        };
        if text.contains(',') {
            return text
                .split(',')
                .map(str::trim)
                .flat_map(|item| range(item).unwrap_or_else(|| vec![item.to_string()]))
                .collect();
        }
        range(text).unwrap_or_else(|| vec![text.to_string()])
    }

    // Split on the commas that are not inside parentheses, then take each piece apart
    // into its parenthesised groups and the plain text between them.
    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0_i32;
    for letter in expression.chars() {
        match letter {
            '(' => {
                depth += 1;
                current.push(letter);
            }
            ')' => {
                depth -= 1;
                current.push(letter);
            }
            ',' if depth == 0 => pieces.push(std::mem::take(&mut current).trim().to_string()),
            _ => current.push(letter),
        }
    }
    if !current.is_empty() {
        pieces.push(current.trim().to_string());
    }

    let mut operators = Vec::new();
    for piece in pieces {
        if !piece.contains('(') && !piece.contains(')') {
            operators.extend(expand(&piece));
            continue;
        }
        let mut rest = piece.as_str();
        while !rest.is_empty() {
            if let Some(inside) = rest.strip_prefix('(') {
                let end = inside.find(')').unwrap_or(inside.len());
                operators.extend(expand(inside[..end].trim()));
                rest = inside.get(end + 1..).unwrap_or("");
            } else {
                let end = rest.find('(').unwrap_or(rest.len());
                let plain = rest[..end].trim().trim_matches(')').trim();
                if !plain.is_empty() {
                    operators.extend(expand(plain));
                }
                rest = &rest[end..];
            }
        }
    }
    Some(operators.into_iter().filter(|id| !id.is_empty()).collect())
}

/// The operators an `oper_expression` names, each a sequence to compose - the last of a
/// sequence applied first. A plain expression is one operator per sequence; a product of
/// groups, `(X0)(1-60)`, is every operator of the second group followed by every one of
/// the first, as the PDBx dictionary defines it: here, each of sixty icosahedral
/// operators and then the move from the point frame into the crystal's.
pub fn operator_sequences(expression: &str) -> Vec<Vec<String>> {
    if let Some(single) = operators_of(expression) {
        return single.into_iter().map(|id| vec![id]).collect();
    }
    // A product: `(a)(b)...`, each group a list or ranges.
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut rest = expression.trim();
    while let Some(inside) = rest.strip_prefix('(') {
        let Some(end) = inside.find(')') else { break };
        groups.push(operators_of(&inside[..end]).unwrap_or_default());
        rest = inside[end + 1..].trim_start();
    }
    if !rest.is_empty() || groups.iter().any(Vec::is_empty) {
        return Vec::new();
    }
    let mut sequences: Vec<Vec<String>> = vec![Vec::new()];
    for group in groups {
        sequences = sequences
            .into_iter()
            .flat_map(|sequence| {
                group.iter().map(move |id| {
                    let mut longer = sequence.clone();
                    longer.push(id.clone());
                    longer
                })
            })
            .collect();
    }
    sequences
}

/// The assemblies an entry describes, in its order: from `_pdbx_struct_assembly`, or -
/// where that table is missing - the ids `_pdbx_struct_assembly_gen` uses.
fn list_assemblies(table: Option<&Loop>, gen: Option<&Loop>) -> Vec<super::model::AssemblyInfo> {
    let mut listed: Vec<super::model::AssemblyInfo> = Vec::new();
    if let Some(table) = table {
        for row in &table.rows {
            let id = table
                .get(row, "_pdbx_struct_assembly.id")
                .as_str()
                .trim()
                .to_string();
            if id.is_empty() || listed.iter().any(|known| known.id == id) {
                continue;
            }
            let details = table.get(row, "_pdbx_struct_assembly.details");
            let details = details.as_str().trim();
            listed.push(super::model::AssemblyInfo {
                id,
                details: if details == "?" {
                    String::new()
                } else {
                    details.to_string()
                },
                count: table
                    .get(row, "_pdbx_struct_assembly.oligomeric_count")
                    .as_str()
                    .trim()
                    .parse()
                    .ok(),
            });
        }
    }
    if let Some(gen) = gen {
        for row in &gen.rows {
            let id = gen
                .get(row, "_pdbx_struct_assembly_gen.assembly_id")
                .as_str()
                .trim()
                .to_string();
            if !id.is_empty() && !listed.iter().any(|known| known.id == id) {
                listed.push(super::model::AssemblyInfo {
                    id,
                    details: String::new(),
                    count: None,
                });
            }
        }
    }
    listed
}

/// Makes the entry's first biological assembly out of its deposited atoms.
///
/// The way StructCooker builds one (`build_assembly_dict`): `_pdbx_struct_assembly_gen`
/// says, for each assembly, which chains take part and by which operators; each operator
/// is a rotation and a translation in `_pdbx_struct_oper_list`; a chain is copied once
/// per operator, moved by it, and the copy is called `asym_id_oper_id`. Chains the
/// assembly does not name are not in it. An entry that states no assembly - a prediction,
/// most NMR ensembles - is its deposited chains, each under operator 1, which is also
/// what is returned when the records name nothing that can be built.
///
/// Returns the atoms and the id of the assembly they are, or the deposited atoms and None
/// when the records name nothing that can be built.
fn build_assembly(
    atoms: Vec<RawAtom>,
    gen: Option<&Loop>,
    opers: Option<&Loop>,
    assembly: &str,
) -> (Vec<RawAtom>, Option<String>) {
    let (Some(gen), Some(opers)) = (gen, opers) else {
        return (atoms, None);
    };

    // Which operators each chain is copied by, in the order the chains are first named.
    let mut order: Vec<String> = Vec::new();
    let mut copies: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for row in &gen.rows {
        if gen
            .get(row, "_pdbx_struct_assembly_gen.assembly_id")
            .as_str()
            != assembly
        {
            continue;
        }
        let chains = gen.get(row, "_pdbx_struct_assembly_gen.asym_id_list");
        let expression = gen.get(row, "_pdbx_struct_assembly_gen.oper_expression");
        if chains.as_str().is_empty() || expression.as_str().is_empty() {
            continue;
        }
        let operators = operator_sequences(expression.as_str());
        if operators.is_empty() {
            continue;
        }
        for chain in chains
            .as_str()
            .split(',')
            .map(str::trim)
            .filter(|chain| !chain.is_empty())
        {
            if !copies.contains_key(chain) {
                order.push(chain.to_string());
            }
            copies
                .entry(chain.to_string())
                .or_default()
                .extend(operators.iter().cloned());
        }
    }

    // Each operator as a rotation and a translation.
    let mut moves: HashMap<String, ([[f32; 3]; 3], [f32; 3])> = HashMap::new();
    for row in &opers.rows {
        let id = opers
            .get(row, "_pdbx_struct_oper_list.id")
            .as_str()
            .to_string();
        let number = |tag: String| opers.get(row, &tag).parse_f32();
        let mut matrix = [[0.0_f32; 3]; 3];
        let mut vector = [0.0_f32; 3];
        let mut whole = true;
        for i in 0..3 {
            for (j, cell) in matrix[i].iter_mut().enumerate() {
                match number(format!(
                    "_pdbx_struct_oper_list.matrix[{}][{}]",
                    i + 1,
                    j + 1
                )) {
                    Some(value) => *cell = value,
                    None => whole = false,
                }
            }
            match number(format!("_pdbx_struct_oper_list.vector[{}]", i + 1)) {
                Some(value) => vector[i] = value,
                None => whole = false,
            }
        }
        if whole {
            moves.insert(id, (matrix, vector));
        }
    }

    // The deposited atoms of each chain, in file order.
    let mut of_chain: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, atom) in atoms.iter().enumerate() {
        let label = if atom.label_asym_id.is_empty() {
            atom.chain_id.as_str()
        } else {
            atom.label_asym_id.as_str()
        };
        of_chain.entry(label).or_default().push(index);
    }

    let mut built: Vec<RawAtom> = Vec::new();
    for chain in &order {
        let Some(members) = of_chain.get(chain.as_str()) else {
            continue;
        };
        for sequence in &copies[chain] {
            // Composed right to left: the last operator of the sequence moves the atoms
            // first.
            let mut matrix = [[1.0_f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            let mut vector = [0.0_f32; 3];
            let mut whole = true;
            for id in sequence.iter().rev() {
                let Some((m, v)) = moves.get(id) else {
                    whole = false;
                    break;
                };
                let mut product = [[0.0_f32; 3]; 3];
                let mut moved = [0.0_f32; 3];
                for i in 0..3 {
                    for j in 0..3 {
                        product[i][j] = (0..3).map(|k| m[i][k] * matrix[k][j]).sum();
                    }
                    moved[i] = (0..3).map(|k| m[i][k] * vector[k]).sum::<f32>() + v[i];
                }
                matrix = product;
                vector = moved;
            }
            if !whole {
                continue;
            }
            let operator = sequence.join("x");
            let label = format!("{chain}{}{operator}", super::model::OPERATOR_MARK);
            for &index in members {
                let mut atom = atoms[index].clone();
                let [x, y, z] = atom.position;
                atom.position = [
                    matrix[0][0] * x + matrix[0][1] * y + matrix[0][2] * z + vector[0],
                    matrix[1][0] * x + matrix[1][1] * y + matrix[1][2] * z + vector[1],
                    matrix[2][0] * x + matrix[2][1] * y + matrix[2][2] * z + vector[2],
                ];
                atom.label_asym_id = label.clone();
                built.push(atom);
            }
        }
    }
    // Records that name no chain of this file, or only operators that are products: the
    // deposited coordinates are the best that can be said.
    if built.is_empty() {
        return (atoms, None);
    }
    (built, Some(assembly.to_string()))
}

/// The category of a tag: `_struct_conn` for `_struct_conn.id`.
fn category_of(tag: &str) -> &str {
    tag.split('.').next().unwrap_or(tag)
}

/// The file's own component dictionary: which atoms of each component are bonded.
///
/// `_chem_comp.type` comes from a different loop and says how the component links into a
/// polymer ("L-peptide linking", "DNA linking", "non-polymer"), which is what decides
/// whether consecutive residues get a backbone bond.
fn read_templates(bonds: Option<&Loop>, kinds: Option<&Loop>) -> HashMap<String, Template> {
    let mut linking: HashMap<String, Linking> = HashMap::new();
    if let Some(table) = kinds {
        for row in &table.rows {
            let id = table.get(row, "_chem_comp.id").as_str().trim().to_string();
            if id.is_empty() {
                continue;
            }
            linking.insert(
                id,
                Linking::from_chem_comp_type(table.get(row, "_chem_comp.type").as_str()),
            );
        }
    }

    let mut templates: HashMap<String, Template> = HashMap::new();
    if let Some(table) = bonds {
        for row in &table.rows {
            let comp = table
                .get(row, "_chem_comp_bond.comp_id")
                .as_str()
                .trim()
                .to_string();
            let a = table
                .get(row, "_chem_comp_bond.atom_id_1")
                .as_str()
                .trim()
                .to_string();
            let b = table
                .get(row, "_chem_comp_bond.atom_id_2")
                .as_str()
                .trim()
                .to_string();
            if comp.is_empty() || a.is_empty() || b.is_empty() {
                continue;
            }
            let aromatic = table
                .get(row, "_chem_comp_bond.pdbx_aromatic_flag")
                .as_str()
                .eq_ignore_ascii_case("y");
            let order = if aromatic {
                BondOrder::Aromatic
            } else {
                match table
                    .get(row, "_chem_comp_bond.value_order")
                    .as_str()
                    .to_ascii_uppercase()
                    .as_str()
                {
                    "DOUB" => BondOrder::Double,
                    "TRIP" => BondOrder::Triple,
                    _ => BondOrder::Single,
                }
            };
            let entry = templates.entry(comp.clone()).or_insert_with(|| Template {
                linking: linking.get(&comp).copied().unwrap_or(Linking::NonPolymer),
                bonds: Vec::new(),
            });
            entry.bonds.push((a, b, order));
        }
    }

    // A component that has a type but no bond loop still needs its linking known, or a
    // polymer of it would come out as a pile of unconnected residues.
    for (id, kind) in linking {
        templates.entry(id).or_insert_with(|| Template {
            linking: kind,
            bonds: Vec::new(),
        });
    }
    templates
}

/// `_struct_asym` joined to `_entity`: which entity each chain is, and what kind.
///
/// Two loops because that is how the archive stores it - an asym points at an entity, and
/// the entity says `polymer`, `non-polymer`, `water` or `branched`.
fn read_entities(
    asyms: Option<&Loop>,
    entities: Option<&Loop>,
) -> HashMap<String, (String, String)> {
    let mut kinds: HashMap<String, String> = HashMap::new();
    if let Some(table) = entities {
        for row in &table.rows {
            let id = table.get(row, "_entity.id").as_str().trim().to_string();
            let kind = table.get(row, "_entity.type").as_str().trim().to_string();
            if !id.is_empty() {
                kinds.insert(id, kind);
            }
        }
    }
    let mut found = HashMap::new();
    if let Some(table) = asyms {
        for row in &table.rows {
            let asym = table
                .get(row, "_struct_asym.id")
                .as_str()
                .trim()
                .to_string();
            let entity = table
                .get(row, "_struct_asym.entity_id")
                .as_str()
                .trim()
                .to_string();
            if asym.is_empty() {
                continue;
            }
            let kind = kinds.get(&entity).cloned().unwrap_or_default();
            found.insert(asym, (entity, kind));
        }
    }
    found
}

/// `_struct_conn`: disulfides, covalent modifications, metal coordination.
///
/// Author ids are used rather than label ids, because those are what the coordinates are
/// keyed by here. Records for a symmetry mate are dropped: the atom they name is not in
/// this model, and bonding to the nearest atom that happens to have the right name would
/// invent a bond across the structure.
fn read_connections(table: &Loop) -> Vec<Connection> {
    let mut found = Vec::new();
    for row in &table.rows {
        let kind = match table
            .get(row, "_struct_conn.conn_type_id")
            .as_str()
            .to_ascii_lowercase()
            .as_str()
        {
            "disulf" => BondKind::Disulfide,
            "covale" => BondKind::Covalent,
            "metalc" => BondKind::Metal,
            // hydrog is a hydrogen bond, which is an interaction rather than a bond, and
            // modres is a note about a residue rather than a join between two.
            _ => continue,
        };

        let symmetry = |tag: &str| {
            let value = table.get(row, tag);
            let value = value.as_str().trim();
            value.is_empty() || value == "?" || value == "." || value == "1_555"
        };
        if !symmetry("_struct_conn.ptnr1_symmetry") || !symmetry("_struct_conn.ptnr2_symmetry") {
            continue;
        }

        let end = |partner: usize| -> Option<AtomRef> {
            let chain = table
                .get(row, &format!("_struct_conn.ptnr{partner}_auth_asym_id"))
                .as_str()
                .trim()
                .to_string();
            let label_chain = table
                .get(row, &format!("_struct_conn.ptnr{partner}_label_asym_id"))
                .as_str()
                .trim()
                .to_string();
            let comp_id = pick(
                table.get(row, &format!("_struct_conn.ptnr{partner}_auth_comp_id")),
                table.get(row, &format!("_struct_conn.ptnr{partner}_label_comp_id")),
            )
            .trim()
            .to_string();
            let atom_name = table
                .get(row, &format!("_struct_conn.ptnr{partner}_label_atom_id"))
                .as_str()
                .trim()
                .to_string();
            let number = table
                .get(row, &format!("_struct_conn.ptnr{partner}_auth_seq_id"))
                .as_str()
                .trim()
                .parse::<i32>()
                .ok();
            // Either chain name will do - a writer may give only the label one - but
            // without a residue number the end names every residue of that component in
            // the chain, and "the first one" is a guess that bonds the wrong pair.
            if (chain.is_empty() && label_chain.is_empty())
                || comp_id.is_empty()
                || atom_name.is_empty()
                || number.is_none()
            {
                return None;
            }
            let insertion = table
                // Lower case, like every tag after the tokenizer: spelled as the
                // dictionary spells it, with `PDB`, this never matched anything.
                .get(
                    row,
                    &format!("_struct_conn.pdbx_ptnr{partner}_pdb_ins_code"),
                )
                .as_str()
                .trim()
                .chars()
                .find(|c| c.is_ascii_alphanumeric());
            Some(AtomRef {
                chain,
                label_chain: (!label_chain.is_empty()).then_some(label_chain),
                number,
                comp_id,
                atom_name,
                insertion_code: insertion,
            })
        };

        if let (Some(a), Some(b)) = (end(1), end(2)) {
            found.push(Connection { kind, a, b });
        }
    }
    found
}

/// Reads a `loop_` header and its rows.
///
/// Row length is the column count, and values are consumed positionally: mmCIF rows are
/// not newline-delimited, and a long row may wrap across several lines.
fn read_loop(tokenizer: &mut Tokenizer<'_>) -> Loop {
    let mut columns = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut current: Vec<Value> = Vec::new();

    // Header: consecutive tags.
    loop {
        let checkpoint = tokenizer.position;
        match tokenizer.next() {
            Token::Tag(tag) => {
                columns.insert(tag.clone(), order.len());
                order.push(tag);
            }
            other => {
                tokenizer.position = checkpoint;
                if matches!(other, Token::End) {
                    return Loop { columns, rows };
                }
                break;
            }
        }
    }

    let width = order.len().max(1);
    loop {
        let checkpoint = tokenizer.position;
        match tokenizer.next() {
            Token::Value(value) => {
                current.push(value);
                if current.len() == width {
                    rows.push(std::mem::take(&mut current));
                }
            }
            Token::End => break,
            // A tag, `loop_`, or `data_` ends the table. Rewinding lets the caller see it.
            _ => {
                tokenizer.position = checkpoint;
                break;
            }
        }
    }
    if !current.is_empty() {
        // A truncated final row is kept rather than dropped; padding it keeps the column
        // indices valid so the atoms that did parse are still usable.
        current.resize(width, Value::Absent);
        rows.push(current);
    }

    Loop { columns, rows }
}

/// The atoms of one model, its conformers chosen, and what else the file held.
#[derive(Default)]
struct ReadAtoms {
    atoms: Vec<RawAtom>,
    /// Every model number, in file order.
    models: Vec<String>,
    /// The model read.
    model: Option<String>,
    alternates: Vec<char>,
    /// Residues with more than one conformer.
    doubled: usize,
}

fn read_atoms(table: &Loop, model: Option<&str>, alternate: Option<char>) -> ReadAtoms {
    // The models first: one is read - the one asked for if the file has it, else the
    // first - for the same reason as PDB: overlapping copies otherwise.
    let mut models: Vec<String> = Vec::new();
    for row in &table.rows {
        let number = table.get(row, "_atom_site.pdbx_pdb_model_num");
        let number = number.as_str();
        if models.last().map(String::as_str) != Some(number)
            && !models.iter().any(|known| known == number)
        {
            models.push(number.to_string());
        }
    }
    let chosen = model
        .filter(|wanted| models.iter().any(|known| known == wanted))
        .map(str::to_string)
        .or_else(|| models.first().cloned());
    let mut candidates: Vec<(RawAtom, Option<char>)> = Vec::with_capacity(table.rows.len());
    let mut conformers = super::model::Conformers::default();

    for row in &table.rows {
        if chosen.as_deref().is_some_and(|chosen| {
            table.get(row, "_atom_site.pdbx_pdb_model_num").as_str() != chosen
        }) {
            continue;
        }

        let alt_loc = match table.get(row, "_atom_site.label_alt_id") {
            Value::Text(label) => label.chars().next(),
            Value::Absent => None,
        };

        let x = table.get(row, "_atom_site.cartn_x").parse_f32();
        let y = table.get(row, "_atom_site.cartn_y").parse_f32();
        let z = table.get(row, "_atom_site.cartn_z").parse_f32();
        let (Some(x), Some(y), Some(z)) = (x, y, z) else {
            continue;
        };

        let name = table
            .get(row, "_atom_site.label_atom_id")
            .as_str()
            .to_string();
        let symbol = table.get(row, "_atom_site.type_symbol");
        let element = match &symbol {
            Value::Text(text) => chem::from_symbol(text),
            // No element column. A two-letter reading of the name is right only for a
            // lone ion, whose atom and component share a name (`CA` in `CA`); everywhere
            // else `CA` is an alpha carbon and `HE` a hydrogen, not calcium and helium.
            Value::Absent => {
                let comp = table.get(row, "_atom_site.label_comp_id");
                let comp = match &comp {
                    Value::Text(_) => comp,
                    Value::Absent => table.get(row, "_atom_site.auth_comp_id"),
                };
                let lone_ion = comp.as_str().trim().eq_ignore_ascii_case(name.trim());
                chem::from_atom_name(&name, lone_ion)
            }
        };

        // Author numbering is what publications and users refer to; label numbering is
        // the internal sequential one. Prefer author, fall back to label.
        let residue_number = table
            .get(row, "_atom_site.auth_seq_id")
            .parse_i32()
            .or_else(|| table.get(row, "_atom_site.label_seq_id").parse_i32())
            .unwrap_or(0);

        let label_asym_id = table
            .get(row, "_atom_site.label_asym_id")
            .as_str()
            .to_string();
        let entity_id = table
            .get(row, "_atom_site.label_entity_id")
            .as_str()
            .to_string();
        let chain_id = {
            let auth = table.get(row, "_atom_site.auth_asym_id");
            let id = if auth.as_str().is_empty() {
                label_asym_id.as_str()
            } else {
                auth.as_str()
            };
            if id.is_empty() {
                "A".to_string()
            } else {
                id.to_string()
            }
        };

        let residue_name = {
            let auth = table.get(row, "_atom_site.auth_comp_id");
            let label = table.get(row, "_atom_site.label_comp_id");
            if auth.as_str().is_empty() {
                label.as_str().to_string()
            } else {
                auth.as_str().to_string()
            }
        };

        let atom = RawAtom {
            name,
            element,
            position: [x, y, z],
            b_factor: table
                .get(row, "_atom_site.b_iso_or_equiv")
                .parse_f32()
                .unwrap_or(0.0),
            occupancy: table
                .get(row, "_atom_site.occupancy")
                .parse_f32()
                .unwrap_or(1.0),
            residue_name,
            residue_number,
            insertion_code: table
                .get(row, "_atom_site.pdbx_pdb_ins_code")
                .as_str()
                .chars()
                .next(),
            chain_id,
            is_hetatm: table.get(row, "_atom_site.group_pdb").as_str() == "HETATM",
            label_asym_id,
            entity_id,
        };
        conformers.note(&atom, alt_loc);
        candidates.push((atom, alt_loc));
    }

    // Each residue's conformer: the label asked for where it has it, else its most occupied.
    let atoms = candidates
        .into_iter()
        .filter(|(atom, alt_loc)| conformers.keep(atom, *alt_loc, alternate))
        .map(|(atom, _)| atom)
        .collect();
    ReadAtoms {
        atoms,
        models: models
            .into_iter()
            .filter(|number| !number.is_empty())
            .collect(),
        model: chosen.filter(|number| !number.is_empty()),
        alternates: conformers.labels(),
        doubled: conformers.doubled(),
    }
}

fn apply_struct_conf(structure: &mut Structure, table: &Loop) {
    for row in &table.rows {
        let conf_type = table
            .get(row, "_struct_conf.conf_type_id")
            .as_str()
            .to_ascii_uppercase();
        let kind = if conf_type.starts_with("HELX") {
            SecondaryStructure::Helix
        } else if conf_type.starts_with("STRN") || conf_type.starts_with("SHEET") {
            SecondaryStructure::Sheet
        } else if conf_type.starts_with("TURN") {
            SecondaryStructure::Turn
        } else {
            continue;
        };

        let chain = pick(
            table.get(row, "_struct_conf.beg_auth_asym_id"),
            table.get(row, "_struct_conf.beg_label_asym_id"),
        );
        let from = table
            .get(row, "_struct_conf.beg_auth_seq_id")
            .parse_i32()
            .or_else(|| table.get(row, "_struct_conf.beg_label_seq_id").parse_i32());
        let to = table
            .get(row, "_struct_conf.end_auth_seq_id")
            .parse_i32()
            .or_else(|| table.get(row, "_struct_conf.end_label_seq_id").parse_i32());

        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        let indices: Vec<usize> = structure
            .residue_indices_in_range(&chain, from, to)
            .collect();
        for index in indices {
            structure.residues[index].secondary = kind;
        }
    }
}

/// Marks beta strands from `_struct_sheet_range`.
fn apply_sheet_ranges(structure: &mut Structure, table: &Loop) {
    for row in &table.rows {
        let chain = pick(
            table.get(row, "_struct_sheet_range.beg_auth_asym_id"),
            table.get(row, "_struct_sheet_range.beg_label_asym_id"),
        );
        let from = table
            .get(row, "_struct_sheet_range.beg_auth_seq_id")
            .parse_i32()
            .or_else(|| {
                table
                    .get(row, "_struct_sheet_range.beg_label_seq_id")
                    .parse_i32()
            });
        let to = table
            .get(row, "_struct_sheet_range.end_auth_seq_id")
            .parse_i32()
            .or_else(|| {
                table
                    .get(row, "_struct_sheet_range.end_label_seq_id")
                    .parse_i32()
            });
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        let indices: Vec<usize> = structure
            .residue_indices_in_range(&chain, from, to)
            .collect();
        for index in indices {
            structure.residues[index].secondary = SecondaryStructure::Sheet;
        }
    }
}

/// Author identifiers where present, label identifiers otherwise - the same precedence
/// the atom reader uses, so secondary structure lands on the residues it names.
fn pick(preferred: Value, fallback: Value) -> String {
    if preferred.as_str().is_empty() {
        fallback.as_str().to_string()
    } else {
        preferred.as_str().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
data_TEST
#
_struct.title     'A small test structure'
#
loop_
_struct_conf.conf_type_id
_struct_conf.beg_auth_asym_id
_struct_conf.beg_auth_seq_id
_struct_conf.end_auth_asym_id
_struct_conf.end_auth_seq_id
HELX_P A 1 A 2
#
loop_
_atom_site.group_PDB
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_alt_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.auth_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.occupancy
_atom_site.B_iso_or_equiv
_atom_site.auth_asym_id
_atom_site.pdbx_PDB_model_num
ATOM   1  N N   . MET A 1 1 27.340 24.430 2.614  1.00 92.15 A 1
ATOM   2  C CA  . MET A 1 1 26.266 25.413 2.842  1.00 91.20 A 1
ATOM   3  C C   . MET A 1 1 26.913 26.639 3.531  1.00 90.10 A 1
ATOM   4  N N   . ALA A 2 2 26.335 27.770 3.258  1.00 88.44 A 1
HETATM 5  CA CA  . CA  B 1 101 10.000 10.000 10.000 1.00 20.00 B 1
ATOM   6  N N   . MET A 1 1 99.000 99.000 99.000 1.00 92.15 A 2
#
"#;

    #[test]
    fn reads_coordinates_and_metadata() {
        let structure = parse(SAMPLE, "test.cif").0;
        assert_eq!(structure.title, "A small test structure");
        assert_eq!(structure.atom_count(), 5, "the second model is skipped");
        assert_eq!(structure.position(0), [27.340, 24.430, 2.614]);
        assert_eq!(structure.b_factors[0], 92.15);
        assert_eq!(structure.chains.len(), 2);
        // BioMol's form: the label asym id and the assembly operator, with the author id
        // kept beside it.
        assert_eq!(structure.chains[1].id, "B_1");
        assert_eq!(structure.chains[1].auth_id, "B");
        assert_eq!(structure.chains[1].label_asym_id, "B");
    }

    #[test]
    fn author_numbering_wins_over_label_numbering() {
        // The ligand is label_seq_id 1 but auth_seq_id 101; a user looking for residue
        // 101 must find it there.
        let structure = parse(SAMPLE, "test.cif").0;
        let ligand = structure.residues.last().unwrap();
        assert_eq!(ligand.number, 101);
        assert_eq!(ligand.name, "CA");
    }

    #[test]
    fn element_symbols_are_taken_from_the_type_column() {
        let structure = parse(SAMPLE, "test.cif").0;
        assert_eq!(structure.elements[4], 20, "type_symbol CA is calcium");
        assert_eq!(structure.elements[1], chem::CARBON);
    }

    #[test]
    fn sheet_ranges_come_from_their_own_category() {
        // The PDB records strands in _struct_sheet_range; a parser that looks for STRN in
        // _struct_conf finds none, in any file the archive serves.
        let text = "\
data_X
loop_
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.auth_seq_id
_atom_site.auth_asym_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
C CA MET A 1 A 1.0 0.0 0.0
C CA GLN A 2 A 2.0 0.0 0.0
C CA ILE A 3 A 3.0 0.0 0.0
#
loop_
_struct_sheet_range.sheet_id
_struct_sheet_range.id
_struct_sheet_range.beg_auth_asym_id
_struct_sheet_range.beg_auth_seq_id
_struct_sheet_range.end_auth_asym_id
_struct_sheet_range.end_auth_seq_id
BET 1 A 1 A 2
#
";
        let structure = parse(text, "sheets.cif").0;
        assert_eq!(structure.residues[0].secondary, SecondaryStructure::Sheet);
        assert_eq!(structure.residues[1].secondary, SecondaryStructure::Sheet);
        assert_eq!(structure.residues[2].secondary, SecondaryStructure::Coil);
    }

    #[test]
    fn struct_conf_marks_the_helix() {
        let structure = parse(SAMPLE, "test.cif").0;
        assert_eq!(structure.residues[0].secondary, SecondaryStructure::Helix);
        assert_eq!(structure.residues[1].secondary, SecondaryStructure::Helix);
        assert_eq!(structure.residues[2].secondary, SecondaryStructure::Coil);
    }

    #[test]
    fn quoted_and_multiline_values_survive() {
        let text = "\
data_X
_struct.title
;A title that spans
two lines
;
loop_
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.auth_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
C 'CA' ALA A 1 1.0 2.0 3.0
";
        let structure = parse(text, "quoted.cif").0;
        assert!(structure.title.contains("spans"));
        assert_eq!(structure.atom_count(), 1);
        assert_eq!(structure.atom_names[0], "CA");
    }

    #[test]
    fn a_file_with_no_coordinates_is_empty_not_a_panic() {
        let structure = parse("data_EMPTY\n_struct.title 'nothing here'\n", "empty.cif").0;
        assert!(structure.is_empty());
        assert_eq!(structure.residues.len(), 0);
    }

    #[test]
    fn comments_between_rows_do_not_shift_columns() {
        // '#' comments appear mid-loop in files from several tools; consuming one as a
        // value would offset every subsequent field by one.
        let text = "\
data_X
loop_
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.auth_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
N N MET A 1 1.0 2.0 3.0
# a comment in the middle
C CA MET A 1 4.0 5.0 6.0
";
        let structure = parse(text, "comments.cif").0;
        assert_eq!(structure.atom_count(), 2);
        assert_eq!(structure.position(1), [4.0, 5.0, 6.0]);
    }

    const ATOM_SITE: &str = r#"
loop_
_atom_site.group_PDB
_atom_site.id
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.auth_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.auth_asym_id
ATOM   1 N  CYS A 1 1 0.000 0.000 0.000 A
ATOM   2 CA CYS A 1 1 1.470 0.000 0.000 A
ATOM   3 SG CYS A 1 1 2.000 1.500 0.000 A
ATOM   4 N  CYS A 2 2 6.000 0.000 0.000 A
ATOM   5 CA CYS A 2 2 5.000 1.000 0.000 A
ATOM   6 SG CYS A 2 2 3.500 2.900 0.000 A
ATOM   7 HE HIS A 3 3 9.000 0.000 0.000 A
HETATM 8 CA CA  B . 101 20.00 20.00 20.00 B
#
"#;

    #[test]
    fn a_category_with_one_row_is_read_like_any_other() {
        // One helix and one disulfide: both written as key-value lines, not as loops.
        let text = format!(
            "data_ONE\n#\n\
             _struct_conf.conf_type_id        HELX_P\n\
             _struct_conf.id                  HELX_P1\n\
             _struct_conf.beg_auth_asym_id    A\n\
             _struct_conf.beg_auth_seq_id     1\n\
             _struct_conf.end_auth_asym_id    A\n\
             _struct_conf.end_auth_seq_id     2\n#\n\
             _struct_conn.id                  disulf1\n\
             _struct_conn.conn_type_id        disulf\n\
             _struct_conn.ptnr1_auth_asym_id  A\n\
             _struct_conn.ptnr1_auth_comp_id  CYS\n\
             _struct_conn.ptnr1_auth_seq_id   1\n\
             _struct_conn.ptnr1_label_atom_id SG\n\
             _struct_conn.ptnr2_auth_asym_id  A\n\
             _struct_conn.ptnr2_auth_comp_id  CYS\n\
             _struct_conn.ptnr2_auth_seq_id   2\n\
             _struct_conn.ptnr2_label_atom_id SG\n#\n{ATOM_SITE}"
        );
        let (structure, evidence) = parse(&text, "one.cif");
        assert_eq!(structure.residues[0].secondary, SecondaryStructure::Helix);
        assert_eq!(structure.residues[1].secondary, SecondaryStructure::Helix);
        assert_eq!(structure.residues[2].secondary, SecondaryStructure::Coil);
        assert_eq!(evidence.connections.len(), 1);
        assert_eq!(evidence.connections[0].a.atom_name, "SG");
        assert_eq!(evidence.connections[0].b.number, Some(2));
    }

    #[test]
    fn without_an_element_column_an_alpha_carbon_is_not_calcium() {
        let (structure, _) = parse(&format!("data_NOSYM\n{ATOM_SITE}"), "nosym.cif");
        let element = |index: usize| structure.elements[index];
        assert_eq!(element(1), chem::CARBON, "CA of a cysteine");
        assert_eq!(
            element(6),
            chem::HYDROGEN,
            "HE of a histidine is not helium"
        );
        // The ion, whose atom and component share a name, is the one two-letter reading.
        assert_eq!(chem::symbol(element(7)), "Ca");
    }

    /// A minimal `_atom_site` header, followed by the given rows.
    fn atoms(rows: &str) -> String {
        format!(
            "loop_\n_atom_site.type_symbol\n_atom_site.label_atom_id\n_atom_site.label_comp_id\n\
             _atom_site.label_asym_id\n_atom_site.auth_seq_id\n_atom_site.Cartn_x\n\
             _atom_site.Cartn_y\n_atom_site.Cartn_z\n{rows}"
        )
    }

    #[test]
    fn a_second_data_block_is_not_spliced_into_the_first() {
        let text = format!(
            "data_FIRST\n_struct.title 'First'\n{}#\ndata_SECOND\n_struct.title 'Second'\n{}",
            atoms("N N MET A 1 1.0 2.0 3.0\nC CA MET A 1 2.0 2.0 3.0\n"),
            atoms("O O HOH Z 9 50.0 50.0 50.0\n"),
        );
        let structure = parse(&text, "two.cif").0;
        assert_eq!(structure.entry_id, "FIRST");
        assert_eq!(structure.title, "First");
        assert_eq!(structure.atom_count(), 2);
        assert_eq!(structure.residues[0].name, "MET");
    }

    #[test]
    fn a_leading_block_without_coordinates_does_not_hide_the_one_with_them() {
        let text = format!(
            "data_DICT\n_chem_comp.id LIG\n_chem_comp.type non-polymer\n#\ndata_MODEL\n{}",
            atoms("C C1 LIG A 1 0.0 0.0 0.0\n"),
        );
        let (structure, evidence) = parse(&text, "dict.cif");
        assert_eq!(structure.atom_count(), 1);
        assert_eq!(structure.entry_id, "MODEL");
        assert!(evidence.templates.contains_key("LIG"));
    }

    #[test]
    fn non_finite_numbers_are_not_coordinates_or_b_factors() {
        let text = format!(
            "data_X\n{}",
            atoms("N N MET A 1 nan 2.0 3.0\nC CA MET A 1 inf 2.0 3.0\nC C MET A 1 1.0 -inf 3.0\nO O MET A 1 1.0 2.0 3.0\n"),
        );
        let structure = parse(&text, "nan.cif").0;
        assert_eq!(structure.atom_count(), 1);
        assert_eq!(structure.atom_names[0], "O");
        assert_eq!(Value::Text("NaN".into()).parse_f32(), None);
        assert_eq!(Value::Text("1e3".into()).parse_f32(), Some(1000.0));
    }

    #[test]
    fn crlf_files_read_the_same_as_lf_ones() {
        let lf = format!(
            "data_X\n_struct.title\n;A title\nover two lines\n;\n{}",
            atoms("N N MET A 1 1.0 2.0 3.0\nC 'CA' MET A 1 4.0 5.0 6.0\n"),
        );
        let crlf = lf.replace('\n', "\r\n");
        let (a, b) = (parse(&lf, "lf.cif").0, parse(&crlf, "crlf.cif").0);
        assert_eq!(b.atom_count(), 2);
        assert_eq!(a.positions, b.positions);
        assert_eq!(a.atom_names, b.atom_names);
        assert_eq!(b.atom_names[1], "CA");
        assert!(!b.title.contains('\r'), "{:?}", b.title);
        assert!(b.title.starts_with("A title"));
    }

    #[test]
    fn quotes_inside_values_do_not_end_them() {
        let text = format!(
            "data_X\n_struct.title \"5'-end of the 'loop'\"\n{}",
            atoms("C \"C5'\" DA A 1 1.0 2.0 3.0\nO 'O3'' DA A 1 2.0 2.0 3.0\n"),
        );
        let structure = parse(&text, "quotes.cif").0;
        assert_eq!(structure.title, "5'-end of the 'loop'");
        assert_eq!(structure.atom_names, vec!["C5'", "O3'"]);
    }

    #[test]
    fn unterminated_quotes_and_text_fields_do_not_panic() {
        for text in [
            "data_X\n_struct.title 'never closed",
            "data_X\n_struct.title \"",
            "data_X\n_struct.title\n;never closed\n",
            "data_X\n_struct.title\n;",
            "data_X\nloop_\n",
            "data_X\nloop_\n_atom_site.Cartn_x\n",
            "data_",
            ";",
            "'",
            "",
        ] {
            let structure = parse(text, "broken.cif").0;
            assert!(structure.is_empty(), "{text:?}");
        }
    }

    #[test]
    fn a_truncated_last_row_keeps_the_rows_before_it() {
        let text = format!(
            "data_X\n{}",
            atoms("N N MET A 1 1.0 2.0 3.0\nC CA MET A 1 4.0")
        );
        let structure = parse(&text, "truncated.cif").0;
        assert_eq!(structure.atom_count(), 1, "the cut row has no y or z");
        assert_eq!(structure.position(0), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn missing_values_fall_back_rather_than_becoming_zero() {
        let text = "\
data_X
loop_
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.auth_seq_id
_atom_site.pdbx_PDB_ins_code
_atom_site.label_alt_id
_atom_site.auth_asym_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.B_iso_or_equiv
N N MET A 7 ? ? . ? 1.0 2.0 3.0 ?
C CA MET A 7 ? . . . 2.0 2.0 3.0 15.5
";
        let structure = parse(text, "missing.cif").0;
        assert_eq!(structure.residues.len(), 1);
        let residue = &structure.residues[0];
        assert_eq!(
            residue.number, 7,
            "the label number stands in for a missing author one"
        );
        assert_eq!(residue.insertion_code, None, "? is no insertion code");
        // No author chain: the label asym id names it.
        assert_eq!(structure.chains[0].auth_id, "A");
        assert_eq!(structure.b_factors, vec![0.0, 15.5]);
    }

    #[test]
    fn alternate_locations_keep_one_conformer() {
        let text = "\
data_X
loop_
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_alt_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.auth_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
N N  . SER A 1 0.0 0.0 0.0
C CA . SER A 1 1.5 0.0 0.0
O OG A SER A 1 2.0 1.4 0.0
O OG B SER A 1 2.0 -1.4 0.0
";
        let structure = parse(text, "alt.cif").0;
        assert_eq!(structure.atom_names, vec!["N", "CA", "OG"]);
        assert_eq!(structure.position(2), [2.0, 1.4, 0.0]);
    }

    #[test]
    fn only_the_first_model_of_an_ensemble_is_read() {
        let text = "\
data_X
loop_
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.auth_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.pdbx_PDB_model_num
N N MET A 1 0.0 0.0 0.0 1
C CA MET A 1 1.5 0.0 0.0 1
N N MET A 1 9.0 0.0 0.0 2
C CA MET A 1 9.5 0.0 0.0 2
";
        let structure = parse(text, "nmr.cif").0;
        assert_eq!(structure.atom_count(), 2);
        assert_eq!(structure.position(1), [1.5, 0.0, 0.0]);
    }

    #[test]
    fn a_connection_to_a_symmetry_mate_is_not_a_bond_in_this_model() {
        let text = format!(
            "data_SYM\nloop_\n_struct_conn.id\n_struct_conn.conn_type_id\n\
             _struct_conn.ptnr1_auth_asym_id\n_struct_conn.ptnr1_auth_comp_id\n\
             _struct_conn.ptnr1_auth_seq_id\n_struct_conn.ptnr1_label_atom_id\n\
             _struct_conn.ptnr1_symmetry\n_struct_conn.ptnr2_auth_asym_id\n\
             _struct_conn.ptnr2_auth_comp_id\n_struct_conn.ptnr2_auth_seq_id\n\
             _struct_conn.ptnr2_label_atom_id\n_struct_conn.ptnr2_symmetry\n\
             disulf1 disulf A CYS 1 SG 1_555 A CYS 2 SG 2_655\n\
             disulf2 disulf A CYS 1 SG 1_555 A CYS 2 SG 1_555\n\
             hydrog1 hydrog A CYS 1 N 1_555 A CYS 2 O 1_555\n#\n{ATOM_SITE}"
        );
        let connections = parse(&text, "sym.cif").1.connections;
        assert_eq!(connections.len(), 1, "{connections:?}");
        assert_eq!(connections[0].kind, BondKind::Disulfide);
    }

    #[test]
    fn a_connection_without_a_residue_number_is_not_guessed_at() {
        let text = format!(
            "data_NONUM\n#\n\
             _struct_conn.id                  disulf1\n\
             _struct_conn.conn_type_id        disulf\n\
             _struct_conn.ptnr1_label_asym_id A\n\
             _struct_conn.ptnr1_label_comp_id CYS\n\
             _struct_conn.ptnr1_label_atom_id SG\n\
             _struct_conn.ptnr2_label_asym_id A\n\
             _struct_conn.ptnr2_label_comp_id CYS\n\
             _struct_conn.ptnr2_label_atom_id SG\n#\n{ATOM_SITE}"
        );
        assert!(parse(&text, "nonum.cif").1.connections.is_empty());
    }

    #[test]
    fn an_operator_expression_is_read_the_way_structcooker_reads_it() {
        let ids = |expression: &str| operators_of(expression);
        let list = |ids: &[&str]| Some(ids.iter().map(|id| id.to_string()).collect::<Vec<_>>());
        assert_eq!(ids("1"), list(&["1"]));
        assert_eq!(
            ids("(1,2,6,10,23,24)"),
            list(&["1", "2", "6", "10", "23", "24"])
        );
        assert_eq!(ids("(1-4)"), list(&["1", "2", "3", "4"]));
        assert_eq!(ids("P"), list(&["P"]));
        assert_eq!(ids("P,Q"), list(&["P", "Q"]));
        assert_eq!(ids("1-3,7-8"), list(&["1", "2", "3", "7", "8"]));
        assert_eq!(operators_of("(1-60)").map(|all| all.len()), Some(60));
        // A product of two groups is left out, as StructCooker leaves it out.
        assert_eq!(ids("(X0)(1-60)"), None);
    }

    const DIMER: &str = "data_TEST
#
loop_
_pdbx_struct_assembly_gen.assembly_id
_pdbx_struct_assembly_gen.oper_expression
_pdbx_struct_assembly_gen.asym_id_list
1 1,2 A,B
2 1   A
#
loop_
_pdbx_struct_oper_list.id
_pdbx_struct_oper_list.matrix[1][1]
_pdbx_struct_oper_list.matrix[1][2]
_pdbx_struct_oper_list.matrix[1][3]
_pdbx_struct_oper_list.vector[1]
_pdbx_struct_oper_list.matrix[2][1]
_pdbx_struct_oper_list.matrix[2][2]
_pdbx_struct_oper_list.matrix[2][3]
_pdbx_struct_oper_list.vector[2]
_pdbx_struct_oper_list.matrix[3][1]
_pdbx_struct_oper_list.matrix[3][2]
_pdbx_struct_oper_list.matrix[3][3]
_pdbx_struct_oper_list.vector[3]
1 1 0 0 0   0 1 0 0   0 0 1 0
2 -1 0 0 10  0 -1 0 0  0 0 1 5
#
loop_
_atom_site.group_PDB
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
_atom_site.auth_seq_id
_atom_site.auth_asym_id
_atom_site.pdbx_PDB_model_num
ATOM 1 C CA ALA A 1 1.0 2.0 3.0 1 A 1
ATOM 2 C CA GLY A 2 4.0 2.0 3.0 2 A 1
HETATM 3 ZN ZN ZN B . 1.0 4.0 3.0 101 A 1
HETATM 4 O O HOH C . 9.0 9.0 9.0 201 A 1
";

    #[test]
    fn an_assembly_is_each_named_chain_copied_by_each_of_its_operators() {
        let (structure, _) = parse_unit(DIMER, "dimer.cif", Unit::Assembly);
        let ids: Vec<&str> = structure
            .chains
            .iter()
            .map(|chain| chain.id.as_str())
            .collect();
        // Chains A and B, each under operators 1 and 2; the water's chain is not named by
        // the first assembly and is not in it; the second assembly is not built.
        assert_eq!(ids, ["A_1", "A_2", "B_1", "B_2"]);
        assert!(structure.chains.iter().all(|chain| chain.auth_id == "A"));
        assert_eq!(structure.chains[1].label_asym_id, "A");
        assert_eq!(structure.atom_count(), 6);
        // Operator 2 turns x and y about and moves by (10, 0, 5).
        let moved = structure.position(2);
        assert_eq!(moved, [9.0, -2.0, 8.0]);
    }

    #[test]
    fn as_deposited_the_same_file_is_its_chains_as_written() {
        let (structure, _) = parse_unit(DIMER, "dimer.cif", Unit::Deposited);
        let ids: Vec<&str> = structure
            .chains
            .iter()
            .map(|chain| chain.id.as_str())
            .collect();
        assert_eq!(ids, ["A_1", "B_1", "C_1"]);
        assert_eq!(structure.atom_count(), 4);
    }

    #[test]
    fn a_file_that_states_no_assembly_is_its_deposited_chains() {
        let start = DIMER.find("loop_\n_atom_site").unwrap();
        let text = format!("data_TEST\n#\n{}", &DIMER[start..]);
        let (structure, _) = parse_unit(&text, "none.cif", Unit::Assembly);
        assert_eq!(structure.atom_count(), 4);
        assert_eq!(structure.chains[0].id, "A_1");
    }

    #[test]
    fn a_product_of_operator_groups_is_every_pair_with_the_right_one_first() {
        let sequences = operator_sequences("(X0)(1-3,5)");
        assert_eq!(
            sequences,
            vec![
                vec!["X0".to_string(), "1".into()],
                vec!["X0".to_string(), "2".into()],
                vec!["X0".to_string(), "3".into()],
                vec!["X0".to_string(), "5".into()],
            ]
        );
        assert_eq!(
            operator_sequences("(1-2)"),
            vec![vec!["1".to_string()], vec!["2".to_string()]]
        );
        assert!(operator_sequences("(X0)(").is_empty());
    }

    #[test]
    fn alternate_locations_keep_each_residue_s_most_occupied_or_the_one_asked_for() {
        let text = "data_ALT\nloop_\n_atom_site.group_PDB\n_atom_site.id\n_atom_site.type_symbol\n_atom_site.label_atom_id\n_atom_site.label_alt_id\n_atom_site.label_comp_id\n_atom_site.label_asym_id\n_atom_site.label_seq_id\n_atom_site.Cartn_x\n_atom_site.Cartn_y\n_atom_site.Cartn_z\n_atom_site.occupancy\n_atom_site.auth_seq_id\n_atom_site.auth_asym_id\n_atom_site.pdbx_PDB_model_num\n\
ATOM 1 C CA . SER A 1 0.0 0.0 0.0 1.00 1 A 1\n\
ATOM 2 O OG A SER A 1 1.0 0.0 0.0 0.30 1 A 1\n\
ATOM 3 O OG B SER A 1 0.0 1.0 0.0 0.70 1 A 1\n\
ATOM 4 C CA . SER A 2 3.8 0.0 0.0 1.00 2 A 1\n\
ATOM 5 O OG A SER A 2 4.8 0.0 0.0 0.60 2 A 1\n\
ATOM 6 O OG B SER A 2 3.8 1.0 0.0 0.40 2 A 1\n\
ATOM 7 C CA . SER A 1 9.0 9.0 9.0 1.00 1 A 2\n\
ATOM 8 C CA . SER A 2 9.0 9.0 9.0 1.00 2 A 2\n";
        // Residue 1 keeps B (0.7), residue 2 keeps A (0.6): chosen residue by residue.
        let (s, _) = parse_reading(text, "alt.cif", &Reading::default());
        let og: Vec<[f32; 3]> = (0..s.atom_count())
            .filter(|&a| s.atom_names[a] == "OG")
            .map(|a| s.position(a))
            .collect();
        assert_eq!(og, vec![[0.0, 1.0, 0.0], [4.8, 0.0, 0.0]]);
        assert_eq!(s.versions.alternates, vec!["A", "B"]);
        assert_eq!(s.versions.alternate_residues, 2);
        assert_eq!(s.versions.models, vec!["1", "2"]);
        // Asked for B: B wherever a residue has one.
        let reading = Reading {
            alternate: Some('B'),
            ..Reading::default()
        };
        let (s, _) = parse_reading(text, "alt.cif", &reading);
        let og: Vec<[f32; 3]> = (0..s.atom_count())
            .filter(|&a| s.atom_names[a] == "OG")
            .map(|a| s.position(a))
            .collect();
        assert_eq!(og, vec![[0.0, 1.0, 0.0], [3.8, 1.0, 0.0]]);
        // The second model, which is all there is of it.
        let reading = Reading {
            model: Some("2".into()),
            ..Reading::default()
        };
        let (s, _) = parse_reading(text, "alt.cif", &reading);
        assert_eq!(s.atom_count(), 2);
        assert_eq!(s.versions.model.as_deref(), Some("2"));
    }
}
