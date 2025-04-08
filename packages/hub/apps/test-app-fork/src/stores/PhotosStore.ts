import { sync } from "@tonk/keepsync";
import { create } from "zustand";
import {
  configureSyncedFileSystem,
  addFile,
  removeFile,
  getFile,
  getAllFiles,
} from "@tonk/keepsync";
import type { FileMetadata } from "@tonk/keepsync";

// Define the Photo type
export interface Photo {
  id: string;
  title: string;
  description: string;
  fileHash: string;
  createdAt: number;
  updatedAt: number;
}

// Define the data structure
interface PhotosData {
  photos: Photo[];
}

// Define the store state
interface PhotosState extends PhotosData {
  addPhoto: (title: string, description: string, file: File) => Promise<void>;
  updatePhoto: (id: string, title: string, description: string) => void;
  deletePhoto: (id: string) => Promise<void>;
  getPhotoFile: (fileHash: string) => Promise<Blob | null>;
  getAllPhotoFiles: () => Promise<FileMetadata[] | null>;
}

// Initialize the synced file system
const initSyncedFileSystem = () => {
  configureSyncedFileSystem({
    docId: "photos-app-files",
    dbName: "photos_app_db",
    storeName: "photo-blobs",
  });
};

// Initialize the synced file system once
initSyncedFileSystem();

// Create a synced store for photos
export const usePhotosStore = create<PhotosState>(
  sync(
    (set, get) => ({
      photos: [],

      // Add a new photo
      addPhoto: async (title: string, description: string, file: File) => {
        try {
          // Upload the file to the synced file system
          const fileMetadata = await addFile(file);
          if (!fileMetadata) {
            throw new Error("Failed to upload file");
          }

          const now = Date.now();
          const newPhoto: Photo = {
            id: crypto.randomUUID(),
            title,
            description,
            fileHash: fileMetadata.hash,
            createdAt: now,
            updatedAt: now,
          };

          set((state) => ({
            photos: [...state.photos, newPhoto],
          }));
        } catch (error) {
          console.error("Error adding photo:", error);
          throw error;
        }
      },

      // Update a photo's metadata
      updatePhoto: (id: string, title: string, description: string) => {
        set((state) => ({
          photos: state.photos.map((photo) =>
            photo.id === id
              ? { 
                  ...photo, 
                  title, 
                  description, 
                  updatedAt: Date.now() 
                }
              : photo
          ),
        }));
      },

      // Delete a photo and its file
      deletePhoto: async (id: string) => {
        const photos = get().photos;
        const photoToDelete = photos.find((photo) => photo.id === id);
        
        if (photoToDelete) {
          try {
            // Delete the file from the synced file system
            await removeFile(photoToDelete.fileHash);
            
            // Remove the photo from the store
            set((state) => ({
              photos: state.photos.filter((photo) => photo.id !== id),
            }));
          } catch (error) {
            console.error("Error deleting photo:", error);
            throw error;
          }
        }
      },

      // Get a photo file by hash
      getPhotoFile: async (fileHash: string) => {
        try {
          return await getFile(fileHash);
        } catch (error) {
          console.error("Error getting photo file:", error);
          return null;
        }
      },

      // Get all photo files
      getAllPhotoFiles: async () => {
        try {
          return await getAllFiles();
        } catch (error) {
          console.error("Error getting all photo files:", error);
          return null;
        }
      },
    }),
    {
      // Unique document ID for this store
      docId: "photos-app",
    },
  ),
);