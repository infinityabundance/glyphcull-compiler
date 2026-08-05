//! A minimal DOM implementing html5ever's `TreeSink`.
//!
//! html5ever is a pure tokenizer + tree-construction engine: the tree itself is
//! provided by the caller. We implement a small, arena-backed DOM tuned for the
//! compiler's needs — no `Rc` per node, deterministic iteration, and only the
//! node kinds we consume. Handles are arena indices.
//!
//! The HTML5 tree-construction algorithm (foster parenting, table repair,
//! template contents, implied end tags) is fully handled by html5ever's driver;
//! this type is only the storage it writes into.
//!
//! Handles are arena indices created only by `new_node`; the direct indexing in
//! this module is therefore provably in bounds and is the single choke point for
//! node access (the documented exception to the workspace indexing policy).
#![allow(clippy::indexing_slicing)]

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::HashMap;

use html5ever::interface::tree_builder::{
    ElemName as ElemNameTrait, ElementFlags, NodeOrText, QuirksMode, TreeSink,
};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, ExpandedName, LocalName, Namespace, QualName};

/// A node in the minimal DOM.
pub struct Node {
    /// The node kind.
    pub kind: NodeKind,
    /// Children, in document order.
    pub children: Vec<Handle>,
    /// Parent (None for the document root and detached nodes).
    pub parent: Option<Handle>,
    /// Template contents (only for `<template>` elements).
    pub template_contents: Option<Handle>,
}

/// The kind of a node.
#[allow(missing_docs)]
pub enum NodeKind {
    /// The document root.
    Document,
    /// A doctype declaration.
    Doctype {
        /// Doctype name.
        name: StrTendril,
        /// Public identifier.
        public_id: StrTendril,
        /// System identifier.
        system_id: StrTendril,
    },
    /// A text node.
    Text { contents: StrTendril },
    /// A comment.
    Comment { contents: StrTendril },
    /// An element.
    Element {
        /// Element name (namespace + local name).
        name: QualName,
        /// Attributes in source order.
        attrs: Vec<Attribute>,
        /// Construction flags (template, mathml annotation integration point).
        flags: ElementFlags,
    },
    /// A processing instruction.
    Pi {
        /// Target.
        target: StrTendril,
        /// Data.
        data: StrTendril,
    },
}

/// An arena index into the DOM's node storage.
pub type Handle = usize;

/// The `TreeSink` implementation backing `parse_document`.
pub struct MinimalDom {
    nodes: RefCell<Vec<Node>>,
    document: Handle,
    quirks: Cell<QuirksMode>,
    /// HTML5 parse errors reported by the driver (diagnostics only).
    pub errors: RefCell<Vec<String>>,
}

impl MinimalDom {
    /// Create an empty document.
    pub fn new() -> Self {
        let mut nodes = Vec::with_capacity(32);
        nodes.push(Node {
            kind: NodeKind::Document,
            children: Vec::new(),
            parent: None,
            template_contents: None,
        });
        Self {
            nodes: RefCell::new(nodes),
            document: 0,
            quirks: Cell::new(QuirksMode::NoQuirks),
            errors: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn node(&self, handle: Handle) -> Ref<'_, Node> {
        Ref::map(self.nodes.borrow(), |n| &n[handle])
    }

    fn node_mut(&self, handle: Handle) -> RefMut<'_, Node> {
        RefMut::map(self.nodes.borrow_mut(), |n| &mut n[handle])
    }

    fn new_node(&self, kind: NodeKind) -> Handle {
        let mut nodes = self.nodes.borrow_mut();
        let id = nodes.len();
        nodes.push(Node {
            kind,
            children: Vec::new(),
            parent: None,
            template_contents: None,
        });
        id
    }

    /// Detach `child` from its parent.
    fn detach(&self, child: Handle) {
        let parent = self.node(child).parent;
        if let Some(parent) = parent {
            self.node_mut(parent).children.retain(|c| *c != child);
            self.node_mut(child).parent = None;
        }
    }

    /// True if the node currently has a parent.
    fn has_parent(&self, handle: Handle) -> bool {
        self.node(handle).parent.is_some()
    }
}

impl Default for MinimalDom {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeSink for MinimalDom {
    type Output = MinimalDom;
    type Handle = Handle;
    type ElemName<'a> = ElemNameRef;

    fn finish(self) -> Self::Output {
        self
    }

    fn parse_error(&self, msg: std::borrow::Cow<'static, str>) {
        self.errors.borrow_mut().push(msg.to_string());
    }

    fn get_document(&self) -> Self::Handle {
        self.document
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        let node = self.node(*target);
        match &node.kind {
            NodeKind::Element { name, .. } => ElemNameRef::element(name),
            // The driver guarantees elements only; degrade to an empty name
            // rather than panic (the crate's panic-free policy).
            _ => ElemNameRef::empty(),
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Self::Handle {
        let is_template = flags.template;
        let handle = self.new_node(NodeKind::Element { name, attrs, flags });
        if is_template {
            // Create the template contents document fragment.
            let contents = self.new_node(NodeKind::Document);
            self.node_mut(handle).template_contents = Some(contents);
        }
        handle
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.new_node(NodeKind::Comment { contents: text })
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.new_node(NodeKind::Pi { target, data })
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(node) => {
                self.detach(node);
                self.node_mut(node).parent = Some(*parent);
                self.node_mut(*parent).children.push(node);
            }
            NodeOrText::AppendText(text) => {
                // Merge adjacent text nodes.
                let last = self.node(*parent).children.last().copied();
                if let Some(last) = last {
                    if let NodeKind::Text { contents } = &mut self.node_mut(last).kind {
                        contents.push_tendril(&text);
                        return;
                    }
                }
                let node = self.new_node(NodeKind::Text { contents: text });
                self.node_mut(node).parent = Some(*parent);
                self.node_mut(*parent).children.push(node);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        let parent = if self.has_parent(*element) {
            element
        } else {
            prev_element
        };
        self.append(parent, child);
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let node = self.new_node(NodeKind::Doctype {
            name,
            public_id,
            system_id,
        });
        self.node_mut(node).parent = Some(self.document);
        self.node_mut(self.document).children.push(node);
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        // The driver guarantees this is called with a template element; degrade
        // to the document rather than panic (panic-free policy).
        self.node(*target)
            .template_contents
            .unwrap_or(self.document)
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks.set(mode);
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        let Some(parent) = self.node(*sibling).parent else {
            return;
        };
        match new_node {
            NodeOrText::AppendNode(node) => {
                self.detach(node);
                self.node_mut(node).parent = Some(parent);
                let pos = self
                    .node(parent)
                    .children
                    .iter()
                    .position(|c| *c == *sibling);
                if let Some(pos) = pos {
                    self.node_mut(parent).children.insert(pos, node);
                }
            }
            NodeOrText::AppendText(text) => {
                let pos = self
                    .node(parent)
                    .children
                    .iter()
                    .position(|c| *c == *sibling);
                let Some(pos) = pos else {
                    return;
                };
                // Merge with the previous sibling if it is text.
                if pos > 0 {
                    let prev = self.node(parent).children[pos - 1];
                    if let NodeKind::Text { contents } = &mut self.node_mut(prev).kind {
                        contents.push_tendril(&text);
                        return;
                    }
                }
                let node = self.new_node(NodeKind::Text { contents: text });
                self.node_mut(node).parent = Some(parent);
                self.node_mut(parent).children.insert(pos, node);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let mut node = self.node_mut(*target);
        if let NodeKind::Element {
            attrs: existing, ..
        } = &mut node.kind
        {
            let mut have: HashMap<(LocalName, Option<Namespace>), ()> = HashMap::new();
            for a in existing.iter() {
                have.insert((a.name.local.clone(), Some(a.name.ns.clone())), ());
            }
            for a in attrs {
                if !have.contains_key(&(a.name.local.clone(), Some(a.name.ns.clone()))) {
                    existing.push(a);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.detach(*target);
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let children = std::mem::take(&mut self.node_mut(*node).children);
        for child in &children {
            self.node_mut(*child).parent = Some(*new_parent);
        }
        self.node_mut(*new_parent).children.extend(children);
    }
}

/// The element-name view the driver uses for tag matching. Owns clones so the
/// view is always constructible (the crate forbids panics).
#[derive(Debug)]
pub struct ElemNameRef {
    ns: Namespace,
    local: LocalName,
}

impl ElemNameRef {
    fn element(name: &QualName) -> Self {
        Self {
            ns: name.ns.clone(),
            local: name.local.clone(),
        }
    }

    fn empty() -> Self {
        Self {
            ns: Namespace::from(""),
            local: LocalName::from(""),
        }
    }
}

impl ElemNameTrait for ElemNameRef {
    fn ns(&self) -> &Namespace {
        &self.ns
    }

    fn local_name(&self) -> &LocalName {
        &self.local
    }

    fn expanded(&self) -> ExpandedName<'_> {
        ExpandedName {
            ns: &self.ns,
            local: &self.local,
        }
    }
}
