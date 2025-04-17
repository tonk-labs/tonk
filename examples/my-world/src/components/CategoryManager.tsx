import React, { useState } from "react";
import { useCategoryStore } from "../stores";
import { Tag, Plus, Edit2, Trash2 } from "lucide-react";

const CategoryManager: React.FC = () => {
  const { categories, addCategory, updateCategory, deleteCategory } =
    useCategoryStore();
  const [isCreatingCategory, setIsCreatingCategory] = useState(false);
  const [isEditingCategory, setIsEditingCategory] = useState(false);
  const [newCategoryName, setNewCategoryName] = useState("");
  const [newCategoryColor, setNewCategoryColor] = useState("#007AFF");
  const [editingCategoryId, setEditingCategoryId] = useState<string | null>(
    null,
  );

  // Apple design system colors
  const appleColors = {
    blue: "#007AFF",
    green: "#34C759",
    red: "#FF3B30",
    yellow: "#FFCC00",
    orange: "#FF9500",
    purple: "#AF52DE",
    pink: "#FF2D55",
    teal: "#5AC8FA",
    indigo: "#5856D6",
    gray: {
      light: "#F2F2F7",
      medium: "#E5E5EA",
      dark: "#8E8E93",
    },
    text: {
      primary: "#000000",
      secondary: "#8E8E93",
    },
  };

  // Predefined color options
  const colorOptions = [
    { name: "Blue", value: appleColors.blue },
    { name: "Green", value: appleColors.green },
    { name: "Red", value: appleColors.red },
    { name: "Yellow", value: appleColors.yellow },
    { name: "Orange", value: appleColors.orange },
    { name: "Purple", value: appleColors.purple },
    { name: "Pink", value: appleColors.pink },
    { name: "Teal", value: appleColors.teal },
    { name: "Indigo", value: appleColors.indigo },
    { name: "Gray", value: appleColors.gray.dark },
  ];

  const handleCreateCategory = (e: React.FormEvent) => {
    e.preventDefault();
    if (newCategoryName.trim() === "") return;

    // Pass null explicitly for the icon parameter to avoid undefined
    addCategory(newCategoryName, newCategoryColor, null);
    setNewCategoryName("");
    setNewCategoryColor(appleColors.blue);
    setIsCreatingCategory(false);
  };

  const handleUpdateCategory = (e: React.FormEvent) => {
    e.preventDefault();
    if (!editingCategoryId || newCategoryName.trim() === "") return;

    updateCategory(editingCategoryId, {
      name: newCategoryName,
      color: newCategoryColor,
      icon: null,
    });
    setNewCategoryName("");
    setNewCategoryColor(appleColors.blue);
    setIsEditingCategory(false);
    setEditingCategoryId(null);
  };

  const handleDeleteCategory = (id: string) => {
    deleteCategory(id);
  };

  const startEditingCategory = (category: {
    id: string;
    name: string;
    color: string;
  }) => {
    setEditingCategoryId(category.id);
    setNewCategoryName(category.name);
    setNewCategoryColor(category.color);
    setIsEditingCategory(true);
  };

  return (
    <div
      className="mb-6 rounded-xl overflow-hidden category-manager"
      style={{ backgroundColor: appleColors.gray.light }}
    >
      <div
        className="px-4 py-3 border-b"
        style={{ borderColor: "rgba(0, 0, 0, 0.05)" }}
      >
        <h3
          className="font-semibold flex items-center gap-2"
          style={{ fontSize: "15px" }}
        >
          <Tag className="h-4 w-4" />
          Categories
        </h3>
      </div>

      <div className="p-4">
        {/* Category Creation Form */}
        {isCreatingCategory ? (
          <form onSubmit={handleCreateCategory} className="mb-4">
            <div className="mb-3">
              <label
                htmlFor="new-category-name"
                className="text-sm font-medium"
                style={{ color: appleColors.text.secondary }}
              >
                New Category Name
              </label>
              <input
                id="new-category-name"
                type="text"
                value={newCategoryName}
                onChange={(e) => setNewCategoryName(e.target.value)}
                className="w-full px-3 py-2 border mt-1 focus:outline-none focus:ring-2"
                style={{
                  borderColor: appleColors.gray.medium,
                  borderRadius: "8px",
                  fontSize: "15px",
                }}
                placeholder="Enter category name"
                autoFocus
                required
              />
            </div>

            <div className="mb-3">
              <label
                className="text-sm font-medium"
                style={{ color: appleColors.text.secondary }}
              >
                Color
              </label>
              <div className="grid grid-cols-5 gap-2 mt-1">
                {colorOptions.map((color) => (
                  <button
                    key={color.value}
                    type="button"
                    className={`w-8 h-8 rounded-full ${
                      newCategoryColor === color.value
                        ? "ring-2 ring-offset-2"
                        : ""
                    }`}
                    style={{ backgroundColor: color.value }}
                    onClick={() => setNewCategoryColor(color.value)}
                    aria-label={`Select ${color.name} color`}
                  />
                ))}
              </div>
            </div>

            <div className="flex gap-2">
              <button
                type="button"
                onClick={() => {
                  setIsCreatingCategory(false);
                  setNewCategoryName("");
                  setNewCategoryColor(appleColors.blue);
                }}
                className="flex-1 py-2 px-3 text-sm rounded-lg font-medium"
                style={{
                  backgroundColor: "rgba(142, 142, 147, 0.12)",
                  color: appleColors.text.primary,
                }}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="flex-1 py-2 px-3 text-sm rounded-lg font-medium text-white"
                style={{ backgroundColor: appleColors.blue }}
              >
                Create
              </button>
            </div>
          </form>
        ) : isEditingCategory ? (
          <form onSubmit={handleUpdateCategory} className="mb-4">
            <div className="mb-3">
              <label
                htmlFor="edit-category-name"
                className="text-sm font-medium"
                style={{ color: appleColors.text.secondary }}
              >
                Edit Category Name
              </label>
              <input
                id="edit-category-name"
                type="text"
                value={newCategoryName}
                onChange={(e) => setNewCategoryName(e.target.value)}
                className="w-full px-3 py-2 border mt-1 focus:outline-none focus:ring-2"
                style={{
                  borderColor: appleColors.gray.medium,
                  borderRadius: "8px",
                  fontSize: "15px",
                }}
                placeholder="Enter category name"
                autoFocus
                required
              />
            </div>

            <div className="mb-3">
              <label
                className="text-sm font-medium"
                style={{ color: appleColors.text.secondary }}
              >
                Color
              </label>
              <div className="grid grid-cols-5 gap-2 mt-1">
                {colorOptions.map((color) => (
                  <button
                    key={color.value}
                    type="button"
                    className={`w-8 h-8 rounded-full ${
                      newCategoryColor === color.value
                        ? "ring-2 ring-offset-2"
                        : ""
                    }`}
                    style={{ backgroundColor: color.value }}
                    onClick={() => setNewCategoryColor(color.value)}
                    aria-label={`Select ${color.name} color`}
                  />
                ))}
              </div>
            </div>

            <div className="flex gap-2">
              <button
                type="button"
                onClick={() => {
                  setIsEditingCategory(false);
                  setEditingCategoryId(null);
                  setNewCategoryName("");
                  setNewCategoryColor(appleColors.blue);
                }}
                className="flex-1 py-2 px-3 text-sm rounded-lg font-medium"
                style={{
                  backgroundColor: "rgba(142, 142, 147, 0.12)",
                  color: appleColors.text.primary,
                }}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="flex-1 py-2 px-3 text-sm rounded-lg font-medium text-white"
                style={{ backgroundColor: appleColors.blue }}
              >
                Update
              </button>
            </div>
          </form>
        ) : (
          <button
            onClick={() => setIsCreatingCategory(true)}
            className="mb-4 w-full py-2 px-3 text-sm rounded-lg font-medium flex items-center justify-center gap-2"
            style={{
              backgroundColor: appleColors.blue,
              color: "white",
            }}
          >
            <Plus className="h-4 w-4" />
            Create New Category
          </button>
        )}

        {/* Category List */}
        <div className="space-y-2 max-h-[200px] overflow-y-auto">
          {Object.values(categories).length === 0 ? (
            <div
              className="text-center p-4 text-sm"
              style={{ color: appleColors.text.secondary }}
            >
              No categories yet. Create your first category.
            </div>
          ) : (
            Object.values(categories).map((category) => (
              <div
                key={category.id}
                className="p-3 rounded-lg bg-white flex items-center justify-between"
              >
                <div className="flex items-center gap-2">
                  <div
                    className="w-4 h-4 rounded-full"
                    style={{ backgroundColor: category.color }}
                  ></div>
                  <div className="font-medium">{category.name}</div>
                </div>
                <div className="flex items-center gap-2">
                  {category.createdBy !== "system" && (
                    <>
                      <button
                        onClick={() => startEditingCategory(category)}
                        className="p-1 rounded-full hover:bg-gray-100"
                        aria-label="Edit category"
                      >
                        <Edit2
                          className="h-4 w-4"
                          style={{ color: appleColors.text.secondary }}
                        />
                      </button>
                      <button
                        onClick={() => handleDeleteCategory(category.id)}
                        className="p-1 rounded-full hover:bg-gray-100"
                        aria-label="Delete category"
                      >
                        <Trash2
                          className="h-4 w-4"
                          style={{ color: appleColors.red }}
                        />
                      </button>
                    </>
                  )}
                  {category.createdBy === "system" && (
                    <span
                      className="text-xs"
                      style={{ color: appleColors.text.secondary }}
                    >
                      System
                    </span>
                  )}
                </div>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
};

export default CategoryManager;
