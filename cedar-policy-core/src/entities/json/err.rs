/*
 * Copyright 2022-2023 Amazon.com, Inc. or its affiliates. All Rights Reserved.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::fmt::Display;

use super::SchemaType;
use crate::ast::{
    EntityType, EntityUID, Expr, ExprKind, Name, RestrictedExpr, RestrictedExprError,
};
use crate::extensions::ExtensionFunctionLookupError;
use crate::parser::err::ParseErrors;
use smol_str::SmolStr;
use thiserror::Error;

/// Escape kind
#[derive(Debug)]
pub enum EscapeKind {
    /// Escape `__expr`
    /// Note that `__expr` is deprecated and once it is
    /// removed, this variant will also be removed
    Expr,
    /// Escape `__entity`
    Entity,
    /// Escape `__extn`
    Extension,
}

impl Display for EscapeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Entity => write!(f, "__entity"),
            Self::Expr => write!(f, "__expr"),
            Self::Extension => write!(f, "__extn"),
        }
    }
}

/// Errors thrown during deserialization from JSON
#[derive(Debug, Error)]
pub enum JsonDeserializationError {
    /// Error thrown by the `serde_json` crate
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    /// Contents of an escape failed to parse.
    /// Note that escape `__expr` is deprecated and once it is
    /// removed, `EscapeKind::Expr` will also be removed
    #[error("failed to parse escape `{kind}` with value `{value}`: {errs}")]
    ParseEscape {
        /// Escape kind
        kind: EscapeKind,
        /// Escape value at fault
        value: String,
        /// Parse errors
        errs: ParseErrors,
    },
    /// Restricted expression error
    #[error(transparent)]
    RestrictedExpressionError(#[from] RestrictedExprError),
    /// An error occurred when looking up an extension function
    #[error(transparent)]
    FailedExtensionFunctionLookup(#[from] ExtensionFunctionLookupError),
    /// A field that needs to be a literal entity reference, was some other JSON value
    #[error("{ctx}, expected a literal entity reference, but got `{got}`")]
    ExpectedLiteralEntityRef {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// the expression we got instead
        got: Box<Expr>,
    },
    /// A field that needs to be an extension value, was some other JSON value
    #[error("{ctx}, expected an extension value, but got `{got}`")]
    ExpectedExtnValue {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// the expression we got instead
        got: Box<Expr>,
    },
    /// Contexts need to be records, but we got some other JSON value
    #[error("expected `context` to be a record, but got `{got}`")]
    ExpectedContextToBeRecord {
        /// Expression we got instead
        got: Box<RestrictedExpr>,
    },
    /// Parents of actions should be actions, but this action has a non-action parent
    #[error("action `{uid}` has a non-action parent `{parent}`")]
    ActionParentIsNotAction {
        /// Action entity that had the invalid parent
        uid: EntityUID,
        /// Parent that is invalid
        parent: EntityUID,
    },
    /// Schema-based parsing needed an implicit extension constructor, but no suitable
    /// constructor was found
    #[error("{ctx}, missing extension constructor for {arg_type} -> {return_type}")]
    MissingImpliedConstructor {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// return type of the constructor we were looking for
        return_type: Box<SchemaType>,
        /// argument type of the constructor we were looking for
        arg_type: Box<SchemaType>,
    },
    /// During schema-based parsing, encountered an entity of a type which is
    /// not declared in the schema. Note that this error is only used for non-Action entity types.
    #[error("entity `{uid}` has type `{}` which is not declared in the schema{}",
        &.uid.entity_type(),
        match .suggested_types.as_slice() {
            [] => String::new(),
            [ty] => format!(". Did you mean `{ty}`?"),
            tys => format!(". Did you mean one of {:?}?", tys.iter().map(ToString::to_string).collect::<Vec<String>>())
        }
    )]
    UnexpectedEntityType {
        /// Entity that had the unexpected type
        uid: EntityUID,
        /// Suggested similar entity types that actually are declared in the schema (if any)
        suggested_types: Vec<EntityType>,
    },
    /// During schema-based parsing, encountered an action which was not
    /// declared in the schema
    #[error("found action entity `{uid}`, but it was not declared as an action in the schema")]
    UndeclaredAction {
        /// Action which was not declared in the schema
        uid: EntityUID,
    },
    /// During schema-based parsing, encountered an action whose definition
    /// doesn't precisely match the schema's declaration of that action
    #[error("definition of action `{uid}` does not match its schema declaration")]
    ActionDeclarationMismatch {
        /// Action whose definition mismatched between entity data and schema
        uid: EntityUID,
    },
    /// During schema-based parsing, encountered this attribute on this entity, but that
    /// attribute shouldn't exist on entities of this type
    #[error("attribute `{attr}` on `{uid}` should not exist according to the schema")]
    UnexpectedEntityAttr {
        /// Entity that had the unexpected attribute
        uid: EntityUID,
        /// Name of the attribute that was unexpected
        attr: SmolStr,
    },
    /// During schema-based parsing, encountered this attribute on a record, but
    /// that attribute shouldn't exist on that record
    #[error("{ctx}, record attribute `{record_attr}` should not exist according to the schema")]
    UnexpectedRecordAttr {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// Name of the (Record) attribute which was unexpected
        record_attr: SmolStr,
    },
    /// During schema-based parsing, didn't encounter this attribute of an
    /// entity, but that attribute should have existed
    #[error("expected entity `{uid}` to have an attribute `{attr}`, but it does not")]
    MissingRequiredEntityAttr {
        /// Entity that is missing a required attribute
        uid: EntityUID,
        /// Name of the attribute which was expected
        attr: SmolStr,
    },
    /// During schema-based parsing, didn't encounter this attribute of a
    /// record, but that attribute should have existed
    #[error("{ctx}, expected the record to have an attribute `{record_attr}`, but it does not")]
    MissingRequiredRecordAttr {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// Name of the (Record) attribute which was expected
        record_attr: SmolStr,
    },
    /// During schema-based parsing, the given attribute on the given entity had
    /// a different type than the schema indicated to expect
    #[error("{ctx}, type mismatch: attribute was expected to have type {expected}, but actually has type {actual}")]
    TypeMismatch {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// Type which was expected
        expected: Box<SchemaType>,
        /// Type which was encountered instead
        actual: Box<SchemaType>,
    },
    /// During schema-based parsing, found a set whose elements don't all have the
    /// same type.  This doesn't match any possible schema.
    #[error("{ctx}, set elements have different types: {ty1} and {ty2}")]
    HeterogeneousSet {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// First element type which was found
        ty1: Box<SchemaType>,
        /// Second element type which was found
        ty2: Box<SchemaType>,
    },
    /// During schema-based parsing, found a parent of a type that's not allowed
    /// for that entity
    #[error(
        "{ctx}, `{uid}` is not allowed to have a parent of type `{parent_ty}` according to the schema"
    )]
    InvalidParentType {
        /// Context of this error
        ctx: Box<JsonDeserializationErrorContext>,
        /// Entity that has an invalid parent type
        uid: EntityUID,
        /// Parent type which was invalid
        parent_ty: Box<EntityType>, // boxed to avoid this variant being very large (and thus all JsonDeserializationErrors being large)
    },
}

/// Errors thrown during serialization to JSON
#[derive(Debug, Error)]
pub enum JsonSerializationError {
    /// Error thrown by `serde_json`
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    /// Extension-function calls with 0 arguments are not currently supported in
    /// our JSON format.
    #[error("unsupported call to `{func}`. Extension function calls with 0 arguments are not currently supported in our JSON format")]
    ExtnCall0Arguments {
        /// Name of the function which was called with 0 arguments
        func: Name,
    },
    /// Extension-function calls with 2 or more arguments are not currently
    /// supported in our JSON format.
    #[error("unsupported call to `{func}`. Extension function calls with 2 or more arguments are not currently supported in our JSON format")]
    ExtnCall2OrMoreArguments {
        /// Name of the function which was called with 2 or more arguments
        func: Name,
    },
    /// Encountered a `Record` which can't be serialized to JSON because it
    /// contains a key which is reserved as a JSON escape.
    #[error("record uses reserved key `{key}`")]
    ReservedKey {
        /// Reserved key which was used by the `Record`
        key: SmolStr,
    },
    /// Encountered an `ExprKind` which we didn't expect. Either a case is
    /// missing in `JSONValue::from_expr()`, or an internal invariant was
    /// violated and there is a non-restricted expression in `RestrictedExpr`
    #[error("unexpected restricted expression `{kind:?}`")]
    UnexpectedRestrictedExprKind {
        /// `ExprKind` which we didn't expect to find
        kind: ExprKind,
    },
}

/// Gives information about the context of a JSON deserialization error (e.g.,
/// where we were in the JSON document).
#[derive(Debug, Clone)]
pub enum JsonDeserializationErrorContext {
    /// The error occurred while deserializing the attribute `attr` of an entity.
    EntityAttribute {
        /// Entity where the error occurred
        uid: EntityUID,
        /// Attribute where the error occurred
        attr: SmolStr,
    },
    /// The error occurred while deserializing the `parents` field of an entity.
    EntityParents {
        /// Entity where the error occurred
        uid: EntityUID,
    },
    /// The error occurred while deserializing the `uid` field of an entity.
    EntityUid,
    /// The error occurred while deserializing the `Context`.
    Context,
}

impl std::fmt::Display for JsonDeserializationErrorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EntityAttribute { uid, attr } => write!(f, "in attribute `{attr}` on `{uid}`"),
            Self::EntityParents { uid } => write!(f, "in parents field of `{uid}`"),
            Self::EntityUid => write!(f, "in uid field of <unknown entity>"),
            Self::Context => write!(f, "while parsing context"),
        }
    }
}
